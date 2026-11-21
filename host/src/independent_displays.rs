//! Startup orchestration for independently assigned secondary displays.

use std::{
    thread,
    time::{Duration, Instant},
};

use wallpaper_core::{
    editor_settings::WallpaperEditorSettings, playback_settings::PlaybackSettings,
};
use wallpaper_desktop::discover_desktop_host;
use wallpaper_display::{
    DisplayInfo, DisplayPresentationMode, DisplayProfile, DisplayProfileSet, DisplayRect,
    DisplaySnapshot, SpanBounds, enumerate_displays, virtual_desktop_bounds,
};
use wallpaper_ipc::protocol::{
    Command, PlaybackCommandSender, PlaybackStateEvent, ResponseResult, playback_command_channel,
};
use wallpaper_media::{
    Availability, CatalogStore, DisplayProfileStore, EditorSettingsStore, MediaKind,
    PlaybackSettingsStore, WallpaperEntry, decode_image,
};
use wallpaper_renderer::D3d11Device;

use crate::{
    playback::PlaybackWorker,
    recovery::{ApplySettingsError, DesktopRecoveryWorker},
};

#[derive(Clone, Debug)]
pub(crate) struct ResolvedDisplayAssignment {
    pub(crate) display: DisplayInfo,
    pub(crate) profile: DisplayProfile,
    pub(crate) wallpaper: WallpaperEntry,
    pub(crate) span_bounds: Option<SpanBounds>,
}

pub(crate) struct ResolvedDisplayAssignments {
    pub(crate) primary_bounds: Option<DisplayRect>,
    pub(crate) profiles: Vec<ResolvedDisplayAssignment>,
}

pub(crate) fn load_display_assignments() -> ResolvedDisplayAssignments {
    let snapshot = match enumerate_displays() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(event = "display_assignment_enumeration_failed", error = ?error,
                "Could not enumerate displays for wallpaper assignments");
            return ResolvedDisplayAssignments {
                primary_bounds: None,
                profiles: Vec::new(),
            };
        }
    };
    let primary_bounds = snapshot
        .displays()
        .iter()
        .find(|display| display.is_primary())
        .map(DisplayInfo::bounds);
    let profiles = match DisplayProfileStore::for_current_user().and_then(|store| store.load()) {
        Ok(profiles) => profiles,
        Err(error) => {
            tracing::warn!(event = "display_profiles_load_failed", error = ?error,
                "Display assignments are unavailable");
            return ResolvedDisplayAssignments {
                primary_bounds,
                profiles: Vec::new(),
            };
        }
    };
    let library = match CatalogStore::for_current_user().and_then(|store| store.load()) {
        Ok(library) => library,
        Err(error) => {
            tracing::warn!(event = "wallpaper_catalog_load_failed", error = ?error,
                "Display assignments are unavailable");
            return ResolvedDisplayAssignments {
                primary_bounds,
                profiles: Vec::new(),
            };
        }
    };

    let (mode, selected_profiles) = select_profiles_for_active_mode(&snapshot, &profiles);
    let virtual_bounds = if mode == Some(DisplayPresentationMode::Span) {
        let rectangles: Vec<_> = snapshot
            .displays()
            .iter()
            .map(DisplayInfo::bounds)
            .collect();
        match virtual_desktop_bounds(&rectangles) {
            Ok(bounds) => Some(bounds),
            Err(error) => {
                tracing::error!(event = "span_virtual_bounds_invalid", error = ?error,
                    "Span display geometry is invalid; no display targets will start");
                return ResolvedDisplayAssignments {
                    primary_bounds,
                    profiles: Vec::new(),
                };
            }
        }
    } else {
        None
    };
    let mut assignments = Vec::new();
    for (target_display, profile) in selected_profiles {
        let Some(wallpaper) = library.get_by_raw_id(profile.wallpaper_id()) else {
            tracing::warn!(
                event = "display_wallpaper_missing",
                device_name = target_display.device_name(),
                wallpaper_id = profile.wallpaper_id(),
                "Skipping display with an unknown wallpaper assignment"
            );
            continue;
        };
        if wallpaper.availability() != Availability::Available || !wallpaper.source_path().is_file()
        {
            tracing::warn!(
                event = "display_wallpaper_unavailable",
                device_name = target_display.device_name(),
                wallpaper_id = profile.wallpaper_id(),
                "Skipping display with unavailable source media"
            );
            continue;
        }
        assignments.push(ResolvedDisplayAssignment {
            span_bounds: virtual_bounds.and_then(|virtual_bounds| {
                SpanBounds::new(virtual_bounds, target_display.bounds()).ok()
            }),
            display: target_display,
            profile,
            wallpaper: wallpaper.clone(),
        });
    }
    tracing::info!(
        component = "display",
        event = "display_assignments_resolved",
        active_display_count = snapshot.displays().len(),
        presentation_mode = ?mode,
        display_target_count = assignments.len(),
        primary_bounds = ?primary_bounds,
        "Resolved display presentation targets"
    );
    ResolvedDisplayAssignments {
        primary_bounds,
        profiles: assignments,
    }
}

fn select_profiles_for_active_mode(
    snapshot: &DisplaySnapshot,
    profiles: &DisplayProfileSet,
) -> (
    Option<DisplayPresentationMode>,
    Vec<(DisplayInfo, DisplayProfile)>,
) {
    let primary_mode = snapshot
        .displays()
        .iter()
        .find(|display| display.is_primary())
        .and_then(|display| profiles.get(display.id()))
        .map(DisplayProfile::presentation_mode)
        .filter(|mode| *mode != DisplayPresentationMode::Independent);
    let selected_mode = primary_mode.or_else(|| {
        snapshot.displays().iter().find_map(|display| {
            profiles
                .get(display.id())
                .map(DisplayProfile::presentation_mode)
                .filter(|mode| *mode != DisplayPresentationMode::Independent)
        })
    });

    match selected_mode {
        Some(DisplayPresentationMode::Duplicate) => {
            let duplicate_profiles = snapshot
                .displays()
                .iter()
                .filter_map(|display| {
                    profiles
                        .get(display.id())
                        .filter(|profile| {
                            profile.presentation_mode() == DisplayPresentationMode::Duplicate
                        })
                        .map(|profile| (display, profile))
                })
                .collect::<Vec<_>>();
            let (_, source_profile) = duplicate_profiles
                .iter()
                .copied()
                .find(|(display, _)| display.is_primary())
                .or_else(|| duplicate_profiles.first().copied())
                .expect("selected Duplicate mode has an active source profile");
            let wallpaper_id = source_profile.wallpaper_id();
            for (target_display, profile) in &duplicate_profiles {
                if profile.wallpaper_id() != wallpaper_id {
                    tracing::warn!(
                        component = "display",
                        event = "duplicate_profile_source_conflict",
                        device_name = target_display.device_name(),
                        selected_wallpaper_id = wallpaper_id,
                        ignored_wallpaper_id = profile.wallpaper_id(),
                        "Using the selected duplicate source for every active target"
                    );
                }
            }
            let targets = snapshot
                .displays()
                .iter()
                .map(|display| {
                    let fps_override = profiles
                        .get(display.id())
                        .and_then(DisplayProfile::fps_override);
                    let profile = DisplayProfile::new(
                        display.id().clone(),
                        wallpaper_id,
                        DisplayPresentationMode::Duplicate,
                        fps_override,
                    )
                    .expect("active display IDs and stored FPS overrides are validated");
                    (display.clone(), profile)
                })
                .collect();
            (Some(DisplayPresentationMode::Duplicate), targets)
        }
        Some(DisplayPresentationMode::Span) => {
            let span_sources = snapshot
                .displays()
                .iter()
                .filter_map(|display| {
                    profiles
                        .get(display.id())
                        .filter(|profile| {
                            profile.presentation_mode() == DisplayPresentationMode::Span
                        })
                        .map(|profile| (display, profile))
                })
                .collect::<Vec<_>>();
            let (_, source_profile) = span_sources
                .iter()
                .copied()
                .find(|(display, _)| display.is_primary())
                .or_else(|| span_sources.first().copied())
                .expect("selected Span mode has an active source profile");
            let targets = snapshot
                .displays()
                .iter()
                .filter_map(|display| {
                    let target_fps = profiles
                        .get(display.id())
                        .and_then(DisplayProfile::fps_override)
                        .or_else(|| source_profile.fps_override());
                    DisplayProfile::new(
                        display.id().clone(),
                        source_profile.wallpaper_id(),
                        DisplayPresentationMode::Span,
                        target_fps,
                    )
                    .ok()
                    .map(|profile| (display.clone(), profile))
                })
                .collect();
            (Some(DisplayPresentationMode::Span), targets)
        }
        _ => {
            let targets = select_independent_profiles(snapshot, profiles);
            let mode = (!targets.is_empty()).then_some(DisplayPresentationMode::Independent);
            (mode, targets)
        }
    }
}

fn select_independent_profiles(
    snapshot: &DisplaySnapshot,
    profiles: &DisplayProfileSet,
) -> Vec<(DisplayInfo, DisplayProfile)> {
    snapshot
        .displays()
        .iter()
        .filter_map(|display| {
            let profile = profiles.get(display.id())?;
            (profile.presentation_mode() == DisplayPresentationMode::Independent)
                .then(|| (display.clone(), profile.clone()))
        })
        .collect()
}
fn select_secondary_assignments(
    assignments: &[ResolvedDisplayAssignment],
    primary_bounds: Option<DisplayRect>,
) -> Vec<ResolvedDisplayAssignment> {
    let Some(primary_bounds) = primary_bounds else {
        return Vec::new();
    };
    let mut occupied = vec![primary_bounds];
    let mut selected = Vec::new();
    for assignment in assignments
        .iter()
        .filter(|assignment| !assignment.display.is_primary())
    {
        let bounds = assignment.display.bounds();
        if occupied
            .iter()
            .any(|existing| bounds_overlap(*existing, bounds))
        {
            tracing::warn!(
                component = "display",
                event = "display_target_bounds_overlap",
                device_name = assignment.display.device_name(),
                "Skipping overlapping display target geometry"
            );
            continue;
        }
        occupied.push(bounds);
        selected.push(assignment.clone());
    }
    selected
}

fn bounds_overlap(left: DisplayRect, right: DisplayRect) -> bool {
    left.left() < right.right()
        && left.right() > right.left()
        && left.top() < right.bottom()
        && left.bottom() > right.top()
}

pub(crate) fn present_assigned_primary_image(
    assignment: &ResolvedDisplayAssignment,
    recovery: &DesktopRecoveryWorker,
) -> Result<(), &'static str> {
    let image = decode_image(assignment.wallpaper.source_path()).map_err(|_| "image_decode")?;
    recovery
        .present_image(image, assignment.wallpaper.source_path().to_path_buf())
        .map_err(|_| "image_present")?;
    apply_editor_settings(recovery, assignment);
    Ok(())
}

pub(crate) fn start_assigned_primary_video(
    runtime: &tokio::runtime::Runtime,
    command_sender: &PlaybackCommandSender,
    playback_events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
    recovery: &DesktopRecoveryWorker,
    assignment: &ResolvedDisplayAssignment,
) -> Result<(), &'static str> {
    start_video(
        runtime,
        command_sender,
        playback_events,
        recovery,
        assignment,
    )
}

pub(crate) struct DisplayTargetManager {
    targets: Vec<DisplayTarget>,
    stopped: bool,
}

impl DisplayTargetManager {
    pub(crate) fn start(
        runtime: &tokio::runtime::Runtime,
        assignments: &[ResolvedDisplayAssignment],
        primary_bounds: Option<DisplayRect>,
    ) -> Self {
        let mut targets = Vec::new();
        for assignment in select_secondary_assignments(assignments, primary_bounds) {
            match DisplayTarget::start(runtime, assignment) {
                Ok(target) => targets.push(target),
                Err((device_name, stage)) => tracing::error!(
                    component = "display",
                    event = "display_target_start_failed",
                    %device_name,
                    stage,
                    "Could not start this display; other display playback remains active"
                ),
            }
        }
        Self {
            targets,
            stopped: false,
        }
    }

    pub(crate) fn explorer_exited(&mut self) {
        for target in &mut self.targets {
            let active = target
                .recovery
                .as_ref()
                .is_none_or(DesktopRecoveryWorker::explorer_exited);
            if !active {
                target.disable("explorer_exited");
            }
        }
    }

    pub(crate) fn explorer_restarted(&mut self) {
        for target in &mut self.targets {
            let active = target
                .recovery
                .as_ref()
                .is_none_or(DesktopRecoveryWorker::explorer_restarted);
            if !active {
                target.disable("explorer_restarted");
            }
        }
    }

    pub(crate) fn stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        self.stopped = true;
        let mut stopped = true;
        for target in &mut self.targets {
            let target_stopped = target.stop_resources();
            stopped = target_stopped && stopped;
        }
        stopped
    }
}

impl Drop for DisplayTargetManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct DisplayTarget {
    device_name: String,
    recovery: Option<DesktopRecoveryWorker>,
    playback: Option<PlaybackWorker>,
    command_sender: Option<PlaybackCommandSender>,
}

impl DisplayTarget {
    fn start(
        runtime: &tokio::runtime::Runtime,
        assignment: ResolvedDisplayAssignment,
    ) -> Result<Self, (String, &'static str)> {
        let device_name = assignment.display.device_name().to_owned();
        let desktop_host =
            discover_desktop_host().map_err(|_| (device_name.clone(), "desktop_discovery"))?;
        let window = desktop_host
            .attach_wallpaper_window_for_display(assignment.display.bounds())
            .map_err(|_| (device_name.clone(), "window_attach"))?;
        let device =
            D3d11Device::create_hardware().map_err(|_| (device_name.clone(), "d3d11_device"))?;
        let media_device = device.device().clone();
        let recovery =
            DesktopRecoveryWorker::start_with_span(window, device, assignment.span_bounds)
                .map_err(|_| (device_name.clone(), "recovery_worker"))?;
        let mut target = Self {
            device_name: device_name.clone(),
            recovery: Some(recovery),
            playback: None,
            command_sender: None,
        };

        let result = match assignment.wallpaper.media_kind() {
            MediaKind::Image => {
                let image = decode_image(assignment.wallpaper.source_path())
                    .map_err(|_| (device_name.clone(), "image_decode"))?;
                target
                    .recovery
                    .as_ref()
                    .expect("recovery exists")
                    .present_image(image, assignment.wallpaper.source_path().to_path_buf())
                    .map_err(|_| (device_name.clone(), "image_present"))?;
                apply_editor_settings(
                    target.recovery.as_ref().expect("recovery exists"),
                    &assignment,
                );
                Ok(())
            }
            MediaKind::Video => {
                let (sender, receiver) = playback_command_channel();
                let (worker, mut events) = PlaybackWorker::start(
                    receiver,
                    media_device,
                    target.recovery.as_ref().expect("recovery exists"),
                )
                .map_err(|_| (device_name.clone(), "playback_worker"))?;
                target.playback = Some(worker);
                target.command_sender = Some(sender.clone());
                start_video(
                    runtime,
                    &sender,
                    &mut events,
                    target.recovery.as_ref().expect("recovery exists"),
                    &assignment,
                )
                .map_err(|stage| (device_name.clone(), stage))
            }
        };
        result?;
        tracing::info!(component = "display", event = "display_target_started", %device_name,
            wallpaper_id = assignment.profile.wallpaper_id(), fps_override = ?assignment.profile.fps_override(),
            presentation_mode = ?assignment.profile.presentation_mode(),
            "Display target started");
        Ok(target)
    }
}

impl Drop for DisplayTarget {
    fn drop(&mut self) {
        let _ = self.stop_resources();
    }
}

impl DisplayTarget {
    fn disable(&mut self, stage: &'static str) {
        tracing::error!(component = "display", event = "display_target_disabled",
            device_name = %self.device_name, stage,
            "This display's recovery worker stopped; other display targets remain active");
        let _ = self.stop_resources();
    }

    fn stop_resources(&mut self) -> bool {
        self.command_sender.take();
        if let Some(mut playback) = self.playback.take() {
            if !playback.stop() {
                tracing::error!(component = "display", event = "independent_playback_join_failed",
                    device_name = %self.device_name,
                    "Retaining this display's recovery resources until process exit");
                std::mem::forget(playback);
                if let Some(recovery) = self.recovery.take() {
                    std::mem::forget(recovery);
                }
                return false;
            }
        }
        if let Some(mut recovery) = self.recovery.take() {
            return recovery.stop();
        }
        true
    }
}

fn start_video(
    runtime: &tokio::runtime::Runtime,
    sender: &PlaybackCommandSender,
    events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
    recovery: &DesktopRecoveryWorker,
    assignment: &ResolvedDisplayAssignment,
) -> Result<(), &'static str> {
    let wallpaper_id = assignment.profile.wallpaper_id();
    let source_path = assignment.wallpaper.source_path().to_path_buf();
    let source_text = source_path.to_string_lossy().into_owned();
    match runtime.block_on(sender.dispatch(Command::LoadVideo {
        wallpaper_id,
        path: source_text.clone(),
    })) {
        ResponseResult::PlaybackState(_) => {}
        _ => return Err("video_load_request"),
    }
    let loaded = wait_for_video_load(runtime, events)?;
    if loaded.last_error.is_some() || loaded.wallpaper_id != Some(wallpaper_id) {
        return Err("video_load");
    }

    let mut settings = PlaybackSettingsStore::for_current_user()
        .and_then(|store| store.load(wallpaper_id))
        .unwrap_or_else(|_| {
            PlaybackSettings::default_for(wallpaper_id).expect("nonzero catalog id")
        });
    if let Some(fps) = assignment.profile.fps_override() {
        settings = settings.with_fps_limit(fps).map_err(|_| "fps_override")?;
    }
    match runtime.block_on(sender.dispatch(Command::ApplyPlaybackSettings {
        settings,
        source_path: source_text.clone(),
    })) {
        ResponseResult::PlaybackSettingsApplied => {}
        _ => return Err("playback_settings"),
    }
    if !matches!(
        runtime.block_on(sender.dispatch(Command::Play)),
        ResponseResult::PlaybackState(_)
    ) {
        return Err("video_play");
    }

    let settings = EditorSettingsStore::for_current_user()
        .and_then(|store| store.load(wallpaper_id))
        .unwrap_or_else(|_| {
            WallpaperEditorSettings::default_for(wallpaper_id).expect("nonzero catalog id")
        });
    let frame_sender = recovery.frame_sender();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match frame_sender.apply_editor_settings(settings.clone(), source_path.clone()) {
            Ok(()) => break,
            Err(ApplySettingsError::WallpaperNotActive) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Err("editor_settings"),
        }
    }
    Ok(())
}

fn wait_for_video_load(
    runtime: &tokio::runtime::Runtime,
    events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
) -> Result<PlaybackStateEvent, &'static str> {
    runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = events.borrow().clone();
            if !state.loading_video {
                return Ok(state);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero()
                || tokio::time::timeout(remaining, events.changed())
                    .await
                    .is_err()
            {
                return Err("video_load_timeout");
            }
        }
    })
}

fn apply_editor_settings(recovery: &DesktopRecoveryWorker, assignment: &ResolvedDisplayAssignment) {
    let wallpaper_id = assignment.profile.wallpaper_id();
    let settings = EditorSettingsStore::for_current_user()
        .and_then(|store| store.load(wallpaper_id))
        .unwrap_or_else(|_| {
            WallpaperEditorSettings::default_for(wallpaper_id).expect("nonzero catalog id")
        });
    if recovery
        .frame_sender()
        .apply_editor_settings(settings, assignment.wallpaper.source_path().to_path_buf())
        .is_err()
    {
        tracing::warn!(
            component = "display",
            event = "display_editor_settings_not_applied",
            device_name = assignment.display.device_name(),
            "Could not apply saved editor settings to this display"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wallpaper_display::{DisplayId, DisplayProfileSet, DisplayRect, DisplaySnapshot};
    use wallpaper_media::WallpaperLibrary;

    fn display(id: &str, left: i32, primary: bool) -> DisplayInfo {
        DisplayInfo::new(
            DisplayId::from_device_path(id).unwrap(),
            id,
            "\\\\.\\DISPLAY1",
            DisplayRect::new(left, 0, left + 1920, 1080).unwrap(),
            100,
            None,
            primary,
        )
        .unwrap()
    }

    #[test]
    fn independent_profile_selection_keeps_each_display_assignment_and_fps() {
        let displays = DisplaySnapshot::new(vec![
            display("display-a", 0, true),
            display("display-b", 1920, false),
            display("display-c", 3840, false),
        ])
        .unwrap();
        let profiles = DisplayProfileSet::new(vec![
            DisplayProfile::new(
                DisplayId::from_device_path("display-a").unwrap(),
                11,
                DisplayPresentationMode::Independent,
                Some(30),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-b").unwrap(),
                22,
                DisplayPresentationMode::Independent,
                Some(60),
            )
            .unwrap(),
        ])
        .unwrap();

        let (mode, selected) = select_profiles_for_active_mode(&displays, &profiles);

        assert_eq!(mode, Some(DisplayPresentationMode::Independent));
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].0.bounds().left(), 0);
        assert_eq!(selected[0].1.wallpaper_id(), 11);
        assert_eq!(selected[0].1.fps_override(), Some(30));
        assert_eq!(selected[1].0.bounds().left(), 1920);
        assert_eq!(selected[1].1.wallpaper_id(), 22);
        assert_eq!(selected[1].1.fps_override(), Some(60));
    }

    #[test]
    fn duplicate_mode_fans_primary_wallpaper_to_every_active_display_and_keeps_fps() {
        let displays = DisplaySnapshot::new(vec![
            display("display-a", 0, true),
            display("display-b", 1920, false),
            display("display-c", 3840, false),
        ])
        .unwrap();
        let profiles = DisplayProfileSet::new(vec![
            DisplayProfile::new(
                DisplayId::from_device_path("display-a").unwrap(),
                11,
                DisplayPresentationMode::Duplicate,
                Some(30),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-b").unwrap(),
                22,
                DisplayPresentationMode::Duplicate,
                Some(60),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-c").unwrap(),
                33,
                DisplayPresentationMode::Duplicate,
                None,
            )
            .unwrap(),
        ])
        .unwrap();

        let (mode, selected) = select_profiles_for_active_mode(&displays, &profiles);

        assert_eq!(mode, Some(DisplayPresentationMode::Duplicate));
        assert_eq!(selected.len(), 3);
        assert!(
            selected
                .iter()
                .all(|(_, profile)| profile.wallpaper_id() == 11)
        );
        assert_eq!(
            selected
                .iter()
                .map(|(_, profile)| profile.fps_override())
                .collect::<Vec<_>>(),
            vec![Some(30), Some(60), None]
        );
    }

    #[test]
    fn duplicate_mode_uses_first_active_duplicate_source_when_primary_has_none() {
        let displays = DisplaySnapshot::new(vec![
            display("display-a", 0, true),
            display("display-b", 1920, false),
        ])
        .unwrap();
        let profiles = DisplayProfileSet::new(vec![
            DisplayProfile::new(
                DisplayId::from_device_path("display-a").unwrap(),
                10,
                DisplayPresentationMode::Independent,
                None,
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-b").unwrap(),
                22,
                DisplayPresentationMode::Duplicate,
                Some(24),
            )
            .unwrap(),
        ])
        .unwrap();

        let (mode, selected) = select_profiles_for_active_mode(&displays, &profiles);

        assert_eq!(mode, Some(DisplayPresentationMode::Duplicate));
        assert_eq!(selected.len(), 2);
        assert!(
            selected
                .iter()
                .all(|(_, profile)| profile.wallpaper_id() == 22)
        );
    }

    #[test]
    fn duplicate_fanout_creates_one_secondary_assignment_per_non_overlapping_display() {
        let displays = DisplaySnapshot::new(vec![
            display("display-a", 0, true),
            display("display-b", 1920, false),
            display("display-c", 3840, false),
        ])
        .unwrap();
        let source_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/pixels.png");
        let mut library = WallpaperLibrary::default();
        let wallpaper_id = library
            .add(source_path, "duplicate-source.png", MediaKind::Image)
            .unwrap();
        let wallpaper = library.get(wallpaper_id).unwrap().clone();
        let profiles = DisplayProfileSet::new(vec![
            DisplayProfile::new(
                DisplayId::from_device_path("display-a").unwrap(),
                wallpaper_id.get(),
                DisplayPresentationMode::Duplicate,
                Some(30),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-b").unwrap(),
                wallpaper_id.get(),
                DisplayPresentationMode::Duplicate,
                Some(60),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-c").unwrap(),
                wallpaper_id.get(),
                DisplayPresentationMode::Duplicate,
                Some(24),
            )
            .unwrap(),
        ])
        .unwrap();
        let (_, selected) = select_profiles_for_active_mode(&displays, &profiles);
        let assignments = selected
            .into_iter()
            .map(|(display, profile)| ResolvedDisplayAssignment {
                display,
                profile,
                wallpaper: wallpaper.clone(),
                span_bounds: None,
            })
            .collect::<Vec<_>>();

        let secondary =
            select_secondary_assignments(&assignments, Some(displays.displays()[0].bounds()));

        assert_eq!(secondary.len(), 2);
        assert!(
            secondary
                .iter()
                .all(|assignment| assignment.wallpaper.id() == wallpaper_id)
        );
        assert_eq!(secondary[0].profile.fps_override(), Some(60));
        assert_eq!(secondary[1].profile.fps_override(), Some(24));
    }

    #[test]
    fn span_profile_fans_one_source_across_all_displays_and_keeps_target_fps() {
        let displays = DisplaySnapshot::new(vec![
            display("display-a", 0, true),
            display("display-b", 1920, false),
            display("display-c", 3840, false),
        ])
        .unwrap();
        let profiles = DisplayProfileSet::new(vec![
            DisplayProfile::new(
                DisplayId::from_device_path("display-b").unwrap(),
                22,
                DisplayPresentationMode::Span,
                Some(30),
            )
            .unwrap(),
            DisplayProfile::new(
                DisplayId::from_device_path("display-c").unwrap(),
                33,
                DisplayPresentationMode::Independent,
                Some(60),
            )
            .unwrap(),
        ])
        .unwrap();

        let (mode, selected) = select_profiles_for_active_mode(&displays, &profiles);
        assert_eq!(mode, Some(DisplayPresentationMode::Span));
        assert_eq!(selected.len(), 3);
        assert!(
            selected
                .iter()
                .all(|(_, profile)| profile.presentation_mode() == DisplayPresentationMode::Span)
        );
        assert!(
            selected
                .iter()
                .all(|(_, profile)| profile.wallpaper_id() == 22)
        );
        assert_eq!(selected[0].1.fps_override(), Some(30));
        assert_eq!(selected[1].1.fps_override(), Some(30));
        assert_eq!(selected[2].1.fps_override(), Some(60));
    }

    #[test]
    fn secondary_selection_rejects_clone_or_overlapping_geometry_only() {
        let primary = DisplayRect::new(-1920, 0, 0, 1080).unwrap();
        let clone = DisplayRect::new(-1920, 0, 0, 1080).unwrap();
        let adjacent = DisplayRect::new(0, 0, 1920, 1080).unwrap();
        let overlap = DisplayRect::new(-400, 0, 1500, 1080).unwrap();
        assert!(bounds_overlap(primary, clone));
        assert!(!bounds_overlap(primary, adjacent));
        assert!(bounds_overlap(primary, overlap));
    }
}
