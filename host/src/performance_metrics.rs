use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use wallpaper_ipc::protocol::{
    PerformanceMediaKind, PerformanceMetricsSnapshot, TargetPerformanceMetrics,
};
use windows::core::Interface;

const METRICS_SCHEMA_VERSION: u16 = 1;
const METRICS_INTERVAL: Duration = Duration::from_secs(1);
const MAX_METRIC_TARGETS: usize = 16;
const MAX_DISPLAY_ID_BYTES: usize = 256;

#[derive(Clone, Default)]
pub(crate) struct PerformanceMetricsRegistry(Arc<Mutex<RegistryState>>);

#[derive(Default)]
struct RegistryState {
    targets: BTreeMap<String, TargetPerformanceMetrics>,
    process_working_set_bytes: Option<u64>,
    process_private_bytes: Option<u64>,
    gpu_local_usage_bytes: Option<u64>,
    gpu_local_budget_bytes: Option<u64>,
    gpu_non_local_usage_bytes: Option<u64>,
    gpu_non_local_budget_bytes: Option<u64>,
    sampled_at_unix_ms: u64,
}

impl PerformanceMetricsRegistry {
    pub(crate) fn register_target(&self, display_id: &str, media_kind: PerformanceMediaKind) {
        if !valid_display_id(display_id) {
            return;
        }
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.targets.contains_key(display_id) {
            return;
        }
        if state.targets.len() >= MAX_METRIC_TARGETS {
            return;
        }
        state.targets.insert(
            display_id.to_owned(),
            TargetPerformanceMetrics {
                display_id: display_id.to_owned(),
                media_kind,
                source_fps: None,
                rendered_fps: None,
                frame_interval_ms: None,
                dropped_frames_total: 0,
                decode_latency_ms: None,
                present_latency_ms: None,
            },
        );
        state.sampled_at_unix_ms = unix_time_ms();
    }

    pub(crate) fn update_target(&self, metrics: TargetPerformanceMetrics) {
        if !valid_display_id(&metrics.display_id) {
            return;
        }
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.targets.contains_key(&metrics.display_id)
            || state.targets.len() < MAX_METRIC_TARGETS
        {
            state.targets.insert(metrics.display_id.clone(), metrics);
            state.sampled_at_unix_ms = unix_time_ms();
        }
    }

    pub(crate) fn remove_target(&self, display_id: &str) {
        if let Ok(mut state) = self.0.lock() {
            state.targets.remove(display_id);
            state.sampled_at_unix_ms = unix_time_ms();
        }
    }

    pub(crate) fn set_media_kind(&self, display_id: &str, media_kind: PerformanceMediaKind) {
        if let Ok(mut state) = self.0.lock()
            && let Some(target) = state.targets.get_mut(display_id)
        {
            target.media_kind = media_kind;
            target.source_fps = None;
            target.rendered_fps = None;
            target.frame_interval_ms = None;
            target.decode_latency_ms = None;
            target.present_latency_ms = None;
            state.sampled_at_unix_ms = unix_time_ms();
        }
    }

    fn media_kind(&self, display_id: &str) -> Option<PerformanceMediaKind> {
        self.0.lock().ok().and_then(|state| {
            state
                .targets
                .get(display_id)
                .map(|target| target.media_kind)
        })
    }

    fn update_memory(&self, memory: ProcessMemorySnapshot) {
        if let Ok(mut state) = self.0.lock() {
            state.process_working_set_bytes = memory.process_working_set_bytes;
            state.process_private_bytes = memory.process_private_bytes;
            state.gpu_local_usage_bytes = memory.gpu_local_usage_bytes;
            state.gpu_local_budget_bytes = memory.gpu_local_budget_bytes;
            state.gpu_non_local_usage_bytes = memory.gpu_non_local_usage_bytes;
            state.gpu_non_local_budget_bytes = memory.gpu_non_local_budget_bytes;
            state.sampled_at_unix_ms = unix_time_ms();
        }
    }

    pub(crate) fn snapshot(&self) -> PerformanceMetricsSnapshot {
        let Ok(state) = self.0.lock() else {
            return PerformanceMetricsSnapshot {
                schema_version: METRICS_SCHEMA_VERSION,
                sampled_at_unix_ms: unix_time_ms(),
                process_working_set_bytes: None,
                process_private_bytes: None,
                gpu_local_usage_bytes: None,
                gpu_local_budget_bytes: None,
                gpu_non_local_usage_bytes: None,
                gpu_non_local_budget_bytes: None,
                targets: Vec::new(),
            };
        };
        PerformanceMetricsSnapshot {
            schema_version: METRICS_SCHEMA_VERSION,
            sampled_at_unix_ms: state.sampled_at_unix_ms,
            process_working_set_bytes: state.process_working_set_bytes,
            process_private_bytes: state.process_private_bytes,
            gpu_local_usage_bytes: state.gpu_local_usage_bytes,
            gpu_local_budget_bytes: state.gpu_local_budget_bytes,
            gpu_non_local_usage_bytes: state.gpu_non_local_usage_bytes,
            gpu_non_local_budget_bytes: state.gpu_non_local_budget_bytes,
            targets: state.targets.values().cloned().collect(),
        }
    }
}

pub(crate) struct PerformanceMetricsRecorder {
    display_id: String,
    media_kind: PerformanceMediaKind,
    registry: PerformanceMetricsRegistry,
    window_started: Instant,
    decoded_frames: u64,
    rendered_frames: u64,
    dropped_frames_total: u64,
    last_pts_100ns: Option<i64>,
    frame_interval_ms: Option<f64>,
    decode_latency_total_ms: f64,
    decode_latency_samples: u64,
    present_latency_total_ms: f64,
    present_latency_samples: u64,
}

pub(crate) struct PerformanceMemorySampler {
    stop_sender: Option<SyncSender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PerformanceMemorySampler {
    pub(crate) fn start(
        device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
        registry: PerformanceMetricsRegistry,
    ) -> std::io::Result<Self> {
        let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("wallpaper-performance-memory".to_owned())
            .spawn(move || memory_sampling_loop(device, registry, stop_receiver))?;
        Ok(Self {
            stop_sender: Some(stop_sender),
            thread: Some(thread),
        })
    }

    pub(crate) fn stop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.try_send(());
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::warn!(
                    component = "performance",
                    event = "memory_sampler_panicked",
                    "Performance memory sampler stopped after a panic"
                );
            }
        }
    }
}

impl Drop for PerformanceMemorySampler {
    fn drop(&mut self) {
        self.stop();
    }
}

fn memory_sampling_loop(
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    registry: PerformanceMetricsRegistry,
    stop_receiver: Receiver<()>,
) {
    loop {
        match stop_receiver.recv_timeout(METRICS_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                registry.update_memory(sample_memory(&device));
            }
        }
    }
}

impl PerformanceMetricsRecorder {
    pub(crate) fn new(
        display_id: String,
        media_kind: PerformanceMediaKind,
        registry: PerformanceMetricsRegistry,
    ) -> Self {
        registry.register_target(&display_id, media_kind);
        Self {
            display_id,
            media_kind,
            registry,
            window_started: Instant::now(),
            decoded_frames: 0,
            rendered_frames: 0,
            dropped_frames_total: 0,
            last_pts_100ns: None,
            frame_interval_ms: None,
            decode_latency_total_ms: 0.0,
            decode_latency_samples: 0,
            present_latency_total_ms: 0.0,
            present_latency_samples: 0,
        }
    }

    pub(crate) fn record_source_frame(&mut self, pts_100ns: i64, decode_latency: Duration) {
        self.decoded_frames = self.decoded_frames.saturating_add(1);
        if let Some(previous) = self.last_pts_100ns
            && pts_100ns > previous
        {
            self.frame_interval_ms = Some((pts_100ns - previous) as f64 / 10_000.0);
        }
        self.last_pts_100ns = Some(pts_100ns);
        self.decode_latency_total_ms += decode_latency.as_secs_f64() * 1_000.0;
        self.decode_latency_samples = self.decode_latency_samples.saturating_add(1);
    }

    pub(crate) fn record_present(&mut self, latency: Duration) {
        self.rendered_frames = self.rendered_frames.saturating_add(1);
        self.present_latency_total_ms += latency.as_secs_f64() * 1_000.0;
        self.present_latency_samples = self.present_latency_samples.saturating_add(1);
    }

    pub(crate) fn record_dropped_frames(&mut self, count: u64) {
        self.dropped_frames_total = self.dropped_frames_total.saturating_add(count);
    }

    pub(crate) fn set_media_kind(&mut self, media_kind: PerformanceMediaKind) {
        self.media_kind = media_kind;
        self.registry.set_media_kind(&self.display_id, media_kind);
        self.window_started = Instant::now();
        self.last_pts_100ns = None;
        self.frame_interval_ms = None;
        self.reset_window_counters();
    }

    pub(crate) fn publish_if_due(&mut self, now: Instant) {
        if now.saturating_duration_since(self.window_started) < METRICS_INTERVAL {
            return;
        }
        self.publish_due(now);
    }

    fn publish_due(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.window_started);
        if elapsed < METRICS_INTERVAL {
            return;
        }
        let registered_kind = self
            .registry
            .media_kind(&self.display_id)
            .unwrap_or(self.media_kind);
        if registered_kind != self.media_kind {
            self.media_kind = registered_kind;
            self.window_started = now;
            self.last_pts_100ns = None;
            self.frame_interval_ms = None;
            self.reset_window_counters();
            return;
        }
        if self.media_kind != PerformanceMediaKind::Video {
            self.window_started = now;
            self.reset_window_counters();
            return;
        }
        let seconds = elapsed.as_secs_f64();
        self.registry.update_target(TargetPerformanceMetrics {
            display_id: self.display_id.clone(),
            media_kind: self.media_kind,
            source_fps: Some(self.decoded_frames as f64 / seconds),
            rendered_fps: Some(self.rendered_frames as f64 / seconds),
            frame_interval_ms: self.frame_interval_ms,
            dropped_frames_total: self.dropped_frames_total,
            decode_latency_ms: average_ms(
                self.decode_latency_total_ms,
                self.decode_latency_samples,
            ),
            present_latency_ms: average_ms(
                self.present_latency_total_ms,
                self.present_latency_samples,
            ),
        });
        self.window_started = now;
        self.reset_window_counters();
    }

    fn reset_window_counters(&mut self) {
        self.decoded_frames = 0;
        self.rendered_frames = 0;
        self.decode_latency_total_ms = 0.0;
        self.decode_latency_samples = 0;
        self.present_latency_total_ms = 0.0;
        self.present_latency_samples = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_DISPLAY_ID_BYTES, MAX_METRIC_TARGETS, PerformanceMetricsRecorder,
        PerformanceMetricsRegistry, TargetPerformanceMetrics, dropped_frames_since,
    };
    use std::time::Duration;
    use wallpaper_ipc::protocol::PerformanceMediaKind;

    #[test]
    fn snapshots_publish_on_one_second_windows_and_isolate_targets() {
        let registry = PerformanceMetricsRegistry::default();
        let mut primary = PerformanceMetricsRecorder::new(
            "DISPLAY-primary".to_owned(),
            PerformanceMediaKind::Video,
            registry.clone(),
        );
        let mut secondary = PerformanceMetricsRecorder::new(
            "DISPLAY-secondary".to_owned(),
            PerformanceMediaKind::Video,
            registry.clone(),
        );
        let primary_start = primary.window_started;
        let secondary_start = secondary.window_started;
        primary.record_source_frame(0, Duration::from_millis(4));
        primary.record_source_frame(100_000, Duration::from_millis(6));
        primary.record_present(Duration::from_millis(3));
        primary.record_dropped_frames(2);
        secondary.record_source_frame(0, Duration::from_millis(10));

        primary.publish_due(primary_start + Duration::from_millis(999));
        assert!(
            registry
                .snapshot()
                .targets
                .iter()
                .all(|target| target.source_fps.is_none())
        );

        primary.publish_due(primary_start + Duration::from_secs(1));
        secondary.publish_due(secondary_start + Duration::from_secs(1));
        let snapshot = registry.snapshot();
        let primary = snapshot
            .targets
            .iter()
            .find(|target| target.display_id == "DISPLAY-primary")
            .unwrap();
        let secondary = snapshot
            .targets
            .iter()
            .find(|target| target.display_id == "DISPLAY-secondary")
            .unwrap();
        assert_eq!(primary.source_fps, Some(2.0));
        assert_eq!(primary.rendered_fps, Some(1.0));
        assert_eq!(primary.frame_interval_ms, Some(10.0));
        assert_eq!(primary.decode_latency_ms, Some(5.0));
        assert_eq!(primary.present_latency_ms, Some(3.0));
        assert_eq!(primary.dropped_frames_total, 2);
        assert_eq!(secondary.source_fps, Some(1.0));
        assert_eq!(secondary.rendered_fps, Some(0.0));
        assert_eq!(secondary.decode_latency_ms, Some(10.0));
    }

    #[test]
    fn registry_bounds_target_count_and_identifier_size() {
        let registry = PerformanceMetricsRegistry::default();
        let oversized_id = "x".repeat(MAX_DISPLAY_ID_BYTES + 1);
        registry.register_target(&oversized_id, PerformanceMediaKind::Image);
        registry.update_target(TargetPerformanceMetrics {
            display_id: oversized_id,
            media_kind: PerformanceMediaKind::Video,
            source_fps: Some(30.0),
            rendered_fps: Some(30.0),
            frame_interval_ms: Some(33.3),
            dropped_frames_total: 0,
            decode_latency_ms: Some(1.0),
            present_latency_ms: Some(1.0),
        });
        for index in 0..MAX_METRIC_TARGETS + 2 {
            registry.register_target(&format!("DISPLAY-{index:02}"), PerformanceMediaKind::Image);
        }
        assert_eq!(registry.snapshot().targets.len(), MAX_METRIC_TARGETS);
    }

    #[test]
    fn image_assignment_stays_image_and_clears_video_window_metrics() {
        let registry = PerformanceMetricsRegistry::default();
        let mut recorder = PerformanceMetricsRecorder::new(
            "DISPLAY-primary".to_owned(),
            PerformanceMediaKind::Video,
            registry.clone(),
        );
        let start = recorder.window_started;
        recorder.record_source_frame(0, Duration::from_millis(5));
        registry.set_media_kind("DISPLAY-primary", PerformanceMediaKind::Image);

        recorder.publish_due(start + Duration::from_secs(1));

        let target = registry.snapshot().targets.remove(0);
        assert_eq!(target.media_kind, PerformanceMediaKind::Image);
        assert_eq!(target.source_fps, None);
        assert_eq!(target.rendered_fps, None);
        assert_eq!(target.decode_latency_ms, None);
    }

    #[test]
    fn recorder_initialization_preserves_existing_assignment_media_kind() {
        let registry = PerformanceMetricsRegistry::default();
        registry.register_target("DISPLAY-primary", PerformanceMediaKind::Unknown);
        registry.set_media_kind("DISPLAY-primary", PerformanceMediaKind::Image);

        let mut recorder = PerformanceMetricsRecorder::new(
            "DISPLAY-primary".to_owned(),
            PerformanceMediaKind::Unknown,
            registry.clone(),
        );
        recorder.publish_due(recorder.window_started + Duration::from_secs(1));

        let target = registry.snapshot().targets.remove(0);
        assert_eq!(target.media_kind, PerformanceMediaKind::Image);
        assert_eq!(target.source_fps, None);
        assert_eq!(target.rendered_fps, None);
    }

    #[test]
    fn dropped_frame_total_survives_scheduler_counter_reset() {
        let registry = PerformanceMetricsRegistry::default();
        let mut recorder = PerformanceMetricsRecorder::new(
            "DISPLAY-primary".to_owned(),
            PerformanceMediaKind::Video,
            registry.clone(),
        );
        let window_started = recorder.window_started;
        let mut previous_scheduler_total = 0;

        recorder.record_dropped_frames(dropped_frames_since(3, &mut previous_scheduler_total));
        // A new playback scheduler starts its local dropped counter at zero.
        previous_scheduler_total = 0;
        recorder.record_dropped_frames(dropped_frames_since(2, &mut previous_scheduler_total));
        recorder.publish_due(window_started + Duration::from_secs(1));

        assert_eq!(registry.snapshot().targets[0].dropped_frames_total, 5);
    }
}

fn valid_display_id(display_id: &str) -> bool {
    !display_id.is_empty() && display_id.len() <= MAX_DISPLAY_ID_BYTES
}

fn average_ms(total_ms: f64, samples: u64) -> Option<f64> {
    (samples > 0).then_some(total_ms / samples as f64)
}

pub(crate) fn dropped_frames_since(current_total: u64, previous_total: &mut u64) -> u64 {
    let newly_dropped = current_total.saturating_sub(*previous_total);
    *previous_total = current_total;
    newly_dropped
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[derive(Default)]
struct ProcessMemorySnapshot {
    process_working_set_bytes: Option<u64>,
    process_private_bytes: Option<u64>,
    gpu_local_usage_bytes: Option<u64>,
    gpu_local_budget_bytes: Option<u64>,
    gpu_non_local_usage_bytes: Option<u64>,
    gpu_non_local_budget_bytes: Option<u64>,
}

#[cfg(windows)]
fn sample_memory(
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
) -> ProcessMemorySnapshot {
    use std::mem::size_of;

    use windows::Win32::{
        Graphics::Dxgi::{
            DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
            DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIDevice,
        },
        System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
    };

    let mut result = ProcessMemorySnapshot::default();
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    if unsafe {
        GetProcessMemoryInfo(
            windows::Win32::System::Threading::GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            counters.cb,
        )
    }
    .is_ok()
    {
        result.process_working_set_bytes = Some(counters.WorkingSetSize as u64);
        result.process_private_bytes = Some(counters.PrivateUsage as u64);
    }

    let adapter = device
        .cast::<IDXGIDevice>()
        .and_then(|dxgi| unsafe { dxgi.GetAdapter() })
        .and_then(|adapter| adapter.cast::<IDXGIAdapter3>());
    if let Ok(adapter) = adapter {
        let mut local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        if unsafe { adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local) }
            .is_ok()
        {
            result.gpu_local_usage_bytes = Some(local.CurrentUsage);
            result.gpu_local_budget_bytes = Some(local.Budget);
        }
        let mut non_local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        if unsafe {
            adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, &mut non_local)
        }
        .is_ok()
        {
            result.gpu_non_local_usage_bytes = Some(non_local.CurrentUsage);
            result.gpu_non_local_budget_bytes = Some(non_local.Budget);
        }
    }
    result
}

#[cfg(not(windows))]
fn sample_memory(
    _device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
) -> ProcessMemorySnapshot {
    ProcessMemorySnapshot::default()
}
