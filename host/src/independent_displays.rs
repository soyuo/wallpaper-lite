//! Startup orchestration for independently assigned secondary displays.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread,
    time::{Duration, Instant},
};

use wallpaper_core::{
    editor_settings::WallpaperEditorSettings, playback_settings::PlaybackSettings,
};
use wallpaper_desktop::discover_desktop_host;
use wallpaper_display::{
    DisplayId, DisplayInfo, DisplayPresentationMode, DisplayProfile, DisplayProfileSet,
    DisplayRect, DisplaySnapshot, RefreshRate, SpanBounds, enumerate_displays,
    virtual_desktop_bounds,
};
use wallpaper_ipc::protocol::{
    Command, DisplayTargetFailure, IpcPlaybackState, PlaybackCommandErrorCode,
    PlaybackCommandSender, PlaybackStateEvent, ResponseResult, playback_command_channel,
};
use wallpaper_media::{
    Availability, CatalogStore, DisplayProfileStore, EditorSettingsStore, MediaKind,
    PlaybackSettingsStore, WallpaperEntry, decode_image,
};
use wallpaper_renderer::D3d11Device;

use crate::{
    playback::PlaybackWorker,
    recovery::{ApplySettingsError, DesktopRecoveryWorker, RecoveryFrameSender},
};

pub(crate) struct DisplayRefreshRequest {
    pub(crate) response: SyncSender<Vec<DisplayTargetFailure>>,
}

fn target_failure(device_name: &str, stage: &str) -> DisplayTargetFailure {
    DisplayTargetFailure {
        device_name: device_name.chars().take(64).collect(),
        stage: stage.chars().take(48).collect(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedDisplayAssignment {
    pub(crate) display: DisplayInfo,
    pub(crate) profile: DisplayProfile,
    pub(crate) wallpaper: WallpaperEntry,
    pub(crate) span_bounds: Option<SpanBounds>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrimaryAssignmentIdentity {
    display_id: DisplayId,
    wallpaper_id: u64,
    mode: DisplayPresentationMode,
    fps_override: Option<u16>,
    source_path: PathBuf,
}

impl From<&ResolvedDisplayAssignment> for PrimaryAssignmentIdentity {
    fn from(assignment: &ResolvedDisplayAssignment) -> Self {
        Self {
            display_id: assignment.display.id().clone(),
            wallpaper_id: assignment.profile.wallpaper_id(),
            mode: assignment.profile.presentation_mode(),
            fps_override: assignment.profile.fps_override(),
            source_path: assignment.wallpaper.source_path().to_path_buf(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ResolvedDisplayAssignments {
    pub(crate) primary_bounds: Option<DisplayRect>,
    pub(crate) profiles: Vec<ResolvedDisplayAssignment>,
}

pub(crate) fn load_display_assignments() -> ResolvedDisplayAssignments {
    match try_load_display_assignments() {
        Ok(assignments) => assignments,
        Err(stage) => {
            tracing::warn!(
                event = "display_assignments_load_failed",
                stage,
                "Could not load display assignments; retaining the current output topology"
            );
            ResolvedDisplayAssignments {
                primary_bounds: None,
                profiles: Vec::new(),
            }
        }
    }
}

pub(crate) fn try_load_display_assignments() -> Result<ResolvedDisplayAssignments, &'static str> {
    let snapshot = enumerate_displays().map_err(|error| {
        tracing::warn!(event = "display_assignment_enumeration_failed", error = ?error,
            "Could not enumerate displays for wallpaper assignments");
        "display_enumeration"
    })?;
    let primary_bounds = snapshot
        .displays()
        .iter()
        .find(|display| display.is_primary())
        .map(DisplayInfo::bounds);
    let profiles = DisplayProfileStore::for_current_user()
        .and_then(|store| store.load())
        .map_err(|error| {
            tracing::warn!(event = "display_profiles_load_failed", error = ?error,
                "Display assignments are unavailable");
            "display_profiles"
        })?;
    let library = CatalogStore::for_current_user()
        .and_then(|store| store.load())
        .map_err(|error| {
            tracing::warn!(event = "wallpaper_catalog_load_failed", error = ?error,
                "Display assignments are unavailable");
            "wallpaper_catalog"
        })?;

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
                return Err("span_virtual_bounds");
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
    Ok(ResolvedDisplayAssignments {
        primary_bounds,
        profiles: assignments,
    })
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
    present_assigned_primary_image_with_sender(assignment, &recovery.frame_sender())
}

fn present_assigned_primary_image_with_sender(
    assignment: &ResolvedDisplayAssignment,
    recovery: &RecoveryFrameSender,
) -> Result<(), &'static str> {
    let image = decode_image(assignment.wallpaper.source_path()).map_err(|_| "image_decode")?;
    present_decoded_primary_image_with_sender(assignment, recovery, image)
}

fn present_decoded_primary_image_with_sender(
    assignment: &ResolvedDisplayAssignment,
    recovery: &RecoveryFrameSender,
    image: wallpaper_media::DecodedImage,
) -> Result<(), &'static str> {
    recovery
        .present_image(image, assignment.wallpaper.source_path().to_path_buf())
        .map_err(|_| "image_present")?;
    apply_editor_settings(recovery, assignment);
    Ok(())
}

async fn replace_primary_with_image(
    command_sender: &PlaybackCommandSender,
    recovery: RecoveryFrameSender,
    assignment: ResolvedDisplayAssignment,
) -> Result<(), &'static str> {
    let decode_path = assignment.wallpaper.source_path().to_path_buf();
    let image = tokio::task::spawn_blocking(move || decode_image(decode_path))
        .await
        .map_err(|_| "image_decode_worker")?
        .map_err(|_| "image_decode")?;

    let previous_state = command_sender.dispatch(Command::GetPlaybackState).await;
    let was_playing = match previous_state {
        ResponseResult::PlaybackState(state) => {
            state.wallpaper_id.is_some() && state.state == IpcPlaybackState::Playing
        }
        ResponseResult::Error {
            code: PlaybackCommandErrorCode::PlaybackUnavailable,
        } => false,
        _ => return Err("playback_state"),
    };

    if was_playing
        && !matches!(
            command_sender.dispatch(Command::Pause).await,
            ResponseResult::PlaybackState(_)
        )
    {
        return Err("playback_pause");
    }

    let present_assignment = assignment.clone();
    let present_recovery = recovery.clone();
    let presentation = tokio::task::spawn_blocking(move || {
        present_decoded_primary_image_with_sender(&present_assignment, &present_recovery, image)
    })
    .await
    .unwrap_or(Err("image_present_worker"));

    if let Err(stage) = presentation {
        if was_playing
            && !matches!(
                command_sender.dispatch(Command::Play).await,
                ResponseResult::PlaybackState(_)
            )
        {
            tracing::error!(
                component = "display",
                event = "primary_video_resume_after_image_failure_failed",
                "The prior primary video could not resume after the image replacement failed"
            );
        }
        return Err(stage);
    }

    match command_sender.dispatch(Command::Stop).await {
        ResponseResult::PlaybackState(_)
        | ResponseResult::Error {
            code: PlaybackCommandErrorCode::PlaybackUnavailable,
        } => {}
        _ => tracing::warn!(
            component = "display",
            event = "primary_video_stop_after_image_switch_failed",
            "The new primary image is active, but the previous video source could not be stopped"
        ),
    }
    Ok(())
}

pub(crate) fn start_assigned_primary_video(
    runtime: &tokio::runtime::Runtime,
    command_sender: &PlaybackCommandSender,
    playback_events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
    recovery: &DesktopRecoveryWorker,
    assignment: &ResolvedDisplayAssignment,
) -> Result<(), &'static str> {
    runtime.block_on(start_video(
        command_sender,
        playback_events,
        recovery.frame_sender(),
        assignment.clone(),
    ))
}

enum PrimaryAssignmentCommand {
    Reconcile {
        assignment: Box<ResolvedDisplayAssignment>,
        completion: Option<tokio::sync::oneshot::Sender<Result<(), &'static str>>>,
    },
    Shutdown,
}

pub(crate) struct PrimaryAssignmentReconciler {
    sender: tokio::sync::mpsc::UnboundedSender<PrimaryAssignmentCommand>,
    task: tokio::task::JoinHandle<()>,
}

impl PrimaryAssignmentReconciler {
    pub(crate) fn start(
        initial_assignment: Option<&ResolvedDisplayAssignment>,
        command_sender: PlaybackCommandSender,
        playback_events: tokio::sync::watch::Receiver<PlaybackStateEvent>,
        recovery: RecoveryFrameSender,
    ) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut active_identity = initial_assignment.map(PrimaryAssignmentIdentity::from);
        let task = tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                let (assignment, completion) = match command {
                    PrimaryAssignmentCommand::Reconcile {
                        assignment,
                        completion,
                    } => (*assignment, completion),
                    PrimaryAssignmentCommand::Shutdown => break,
                };
                let candidate_identity = PrimaryAssignmentIdentity::from(&assignment);
                if active_identity.as_ref() == Some(&candidate_identity) {
                    if let Some(completion) = completion {
                        let _ = completion.send(Ok(()));
                    }
                    continue;
                }
                let result = match assignment.wallpaper.media_kind() {
                    MediaKind::Image => {
                        replace_primary_with_image(&command_sender, recovery.clone(), assignment)
                            .await
                    }
                    MediaKind::Video => {
                        let mut events = playback_events.clone();
                        start_video(
                            &command_sender,
                            &mut events,
                            recovery.clone(),
                            assignment.clone(),
                        )
                        .await
                    }
                };
                match result {
                    Ok(()) => active_identity = Some(candidate_identity),
                    Err(stage) => tracing::warn!(
                        component = "display",
                        event = "primary_assignment_reconcile_failed",
                        stage,
                        "The primary display kept its last successfully applied wallpaper"
                    ),
                }
                if let Some(completion) = completion {
                    let _ = completion.send(result);
                }
            }
        });
        Self { sender, task }
    }

    pub(crate) fn reconcile(&self, assignment: ResolvedDisplayAssignment) -> bool {
        self.sender
            .send(PrimaryAssignmentCommand::Reconcile {
                assignment: Box::new(assignment),
                completion: None,
            })
            .is_ok()
    }

    pub(crate) fn reconcile_with_result(
        &self,
        assignment: ResolvedDisplayAssignment,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), &'static str>>, ()> {
        let (completion, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(PrimaryAssignmentCommand::Reconcile {
                assignment: Box::new(assignment),
                completion: Some(completion),
            })
            .map_err(|_| ())?;
        Ok(receiver)
    }

    pub(crate) async fn stop(self) -> bool {
        let _ = self.sender.send(PrimaryAssignmentCommand::Shutdown);
        tokio::time::timeout(Duration::from_secs(25), self.task)
            .await
            .is_ok_and(|result| result.is_ok())
    }
}

struct DisplayTargetSet {
    targets: Vec<DisplayTarget>,
    stopped: bool,
}

impl DisplayTargetSet {
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

    pub(crate) fn reconcile(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        assignments: &ResolvedDisplayAssignments,
    ) -> Vec<DisplayTargetFailure> {
        let mut failures = Vec::new();
        if self.stopped {
            return vec![target_failure("display manager", "stopped")];
        }
        let desired =
            select_secondary_assignments(&assignments.profiles, assignments.primary_bounds);
        let desired_signatures = desired
            .iter()
            .map(DisplayTargetSignature::from_assignment)
            .collect::<Vec<_>>();
        let current = self
            .targets
            .iter()
            .map(|target| (target.signature.clone(), target.is_active()))
            .collect::<Vec<_>>();
        let plan = plan_target_reconciliation(&current, &desired_signatures);

        for (display_id, replacing) in plan
            .start
            .iter()
            .map(|id| (id, false))
            .chain(plan.replace.iter().map(|id| (id, true)))
        {
            let Some(assignment) = desired
                .iter()
                .find(|assignment| assignment.display.id() == display_id)
            else {
                continue;
            };
            let existing_index = replacing
                .then(|| {
                    self.targets
                        .iter()
                        .position(|target| target.signature.display_id == *display_id)
                })
                .flatten();
            let start = || DisplayTarget::start(runtime, assignment.clone());
            let result = if let Some(index) = existing_index {
                replace_after_success(&mut self.targets[index], start)
            } else {
                start()
            };
            match result {
                Ok(mut old) if existing_index.is_some() => {
                    if !old.stop_resources() {
                        tracing::error!(
                            component = "display",
                            event = "display_target_old_resources_retained",
                            display_id = display_id.as_str(),
                            "Replaced target worker did not stop cleanly"
                        );
                        failures.push(target_failure(&old.device_name, "replace_cleanup"));
                    }
                }
                Ok(new_target) => self.targets.push(new_target),
                Err((device_name, stage)) => {
                    tracing::error!(
                        component = "display",
                        event = "display_target_rebuild_failed",
                        %device_name,
                        stage,
                        "Keeping the previous target while this display rebuild is unavailable"
                    );
                    failures.push(target_failure(&device_name, stage));
                }
            }
        }

        for display_id in plan.remove {
            if let Some(index) = self
                .targets
                .iter()
                .position(|target| target.signature.display_id == display_id)
            {
                let mut removed = self.targets.remove(index);
                if !removed.stop_resources() {
                    tracing::error!(
                        component = "display",
                        event = "display_target_remove_failed",
                        display_id = display_id.as_str(),
                        "Removed display target resources could not be joined cleanly"
                    );
                    failures.push(target_failure(&removed.device_name, "remove_cleanup"));
                }
            }
        }
        failures
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

impl Drop for DisplayTargetSet {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) struct DisplayTargetManager {
    commands: Sender<DisplayTargetCommand>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: bool,
}

enum DisplayTargetCommand {
    ExplorerExited,
    ExplorerRestarted,
    Reconcile {
        assignments: ResolvedDisplayAssignments,
        completion: Option<tokio::sync::oneshot::Sender<Vec<DisplayTargetFailure>>>,
    },
    Stop(SyncSender<bool>),
}

impl DisplayTargetManager {
    pub(crate) fn start(
        assignments: &[ResolvedDisplayAssignment],
        primary_bounds: Option<DisplayRect>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let initial_assignments = assignments.to_vec();
        let worker = thread::Builder::new()
            .name("wallpaper-display-targets".to_owned())
            .spawn(move || run_display_target_worker(receiver, initial_assignments, primary_bounds))
            .map_err(|error| {
                tracing::error!(component = "display", event = "display_target_worker_start_failed",
                    error_kind = ?error.kind(), "Could not start display target manager");
            })
            .ok();
        Self {
            commands,
            worker,
            stopped: false,
        }
    }

    pub(crate) fn explorer_exited(&self) {
        self.send(DisplayTargetCommand::ExplorerExited);
    }

    pub(crate) fn explorer_restarted(&self) {
        self.send(DisplayTargetCommand::ExplorerRestarted);
    }

    pub(crate) fn reconcile(&self, assignments: &ResolvedDisplayAssignments) -> bool {
        self.send(DisplayTargetCommand::Reconcile {
            assignments: assignments.clone(),
            completion: None,
        })
    }

    pub(crate) fn reconcile_with_result(
        &self,
        assignments: &ResolvedDisplayAssignments,
    ) -> Result<tokio::sync::oneshot::Receiver<Vec<DisplayTargetFailure>>, ()> {
        let (completion, receiver) = tokio::sync::oneshot::channel();
        self.commands
            .send(DisplayTargetCommand::Reconcile {
                assignments: assignments.clone(),
                completion: Some(completion),
            })
            .map_err(|_| ())?;
        Ok(receiver)
    }

    fn send(&self, command: DisplayTargetCommand) -> bool {
        if self.stopped {
            return false;
        }
        if self.commands.send(command).is_err() {
            tracing::warn!(
                component = "display",
                event = "display_target_worker_unavailable",
                "Display target management is unavailable"
            );
            return false;
        }
        true
    }

    pub(crate) fn stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        self.stopped = true;
        let Some(worker) = self.worker.take() else {
            return true;
        };
        let (response, receiver) = mpsc::sync_channel(1);
        if self
            .commands
            .send(DisplayTargetCommand::Stop(response))
            .is_err()
        {
            return worker.join().is_ok();
        }
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(stopped) => worker.join().is_ok() && stopped,
            Err(_) => {
                tracing::error!(
                    component = "display",
                    event = "display_target_worker_stop_timed_out",
                    "Display target manager did not stop before the bounded shutdown deadline"
                );
                drop(worker);
                false
            }
        }
    }
}

impl Drop for DisplayTargetManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run_display_target_worker(
    receiver: Receiver<DisplayTargetCommand>,
    initial_assignments: Vec<ResolvedDisplayAssignment>,
    primary_bounds: Option<DisplayRect>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(component = "display", event = "display_target_runtime_start_failed",
                error_kind = ?error.kind(), "Could not initialize the display target runtime");
            return;
        }
    };
    let mut targets = DisplayTargetSet::start(&runtime, &initial_assignments, primary_bounds);
    while let Ok(command) = receiver.recv() {
        match command {
            DisplayTargetCommand::ExplorerExited => targets.explorer_exited(),
            DisplayTargetCommand::ExplorerRestarted => targets.explorer_restarted(),
            DisplayTargetCommand::Reconcile {
                assignments,
                completion,
            } => {
                let failures = targets.reconcile(&runtime, &assignments);
                if let Some(completion) = completion {
                    let _ = completion.send(failures);
                }
            }
            DisplayTargetCommand::Stop(response) => {
                let stopped = targets.stop();
                let _ = response.send(stopped);
                break;
            }
        }
    }
}

struct DisplayTarget {
    device_name: String,
    signature: DisplayTargetSignature,
    media_kind: MediaKind,
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
        let signature = DisplayTargetSignature::from_assignment(&assignment);
        let media_kind = assignment.wallpaper.media_kind();
        let desktop_host =
            discover_desktop_host().map_err(|_| (device_name.clone(), "desktop_discovery"))?;
        let window = desktop_host
            .attach_wallpaper_window_for_display(assignment.display.bounds())
            .map_err(|_| (device_name.clone(), "window_attach"))?;
        let device =
            D3d11Device::create_hardware().map_err(|_| (device_name.clone(), "d3d11_device"))?;
        let media_device = device.device().clone();
        let recovery = DesktopRecoveryWorker::start_with_geometry(
            window,
            device,
            Some(assignment.display.bounds()),
            assignment.span_bounds,
        )
        .map_err(|_| (device_name.clone(), "recovery_worker"))?;
        let mut target = Self {
            device_name: device_name.clone(),
            signature,
            media_kind,
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
                    &target
                        .recovery
                        .as_ref()
                        .expect("recovery exists")
                        .frame_sender(),
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
                runtime
                    .block_on(start_video(
                        &sender,
                        &mut events,
                        target
                            .recovery
                            .as_ref()
                            .expect("recovery exists")
                            .frame_sender(),
                        assignment.clone(),
                    ))
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

impl DisplayTarget {
    fn is_active(&self) -> bool {
        self.recovery.is_some() && (self.media_kind != MediaKind::Video || self.playback.is_some())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DisplayTargetSignature {
    display_id: DisplayId,
    device_name: String,
    bounds: DisplayRect,
    dpi_scale_percent: u32,
    refresh_rate: Option<RefreshRate>,
    span_bounds: Option<SpanBounds>,
    wallpaper_id: u64,
    presentation_mode: DisplayPresentationMode,
    fps_override: Option<u16>,
    source_path: PathBuf,
}

impl DisplayTargetSignature {
    fn from_assignment(assignment: &ResolvedDisplayAssignment) -> Self {
        Self {
            display_id: assignment.display.id().clone(),
            device_name: assignment.display.device_name().to_owned(),
            bounds: assignment.display.bounds(),
            dpi_scale_percent: assignment.display.dpi_scale_percent(),
            refresh_rate: assignment.display.refresh_rate(),
            span_bounds: assignment.span_bounds,
            wallpaper_id: assignment.profile.wallpaper_id(),
            presentation_mode: assignment.profile.presentation_mode(),
            fps_override: assignment.profile.fps_override(),
            source_path: assignment.wallpaper.source_path().to_path_buf(),
        }
    }
}

#[derive(Default)]
struct TargetReconciliationPlan {
    keep: Vec<DisplayId>,
    start: Vec<DisplayId>,
    replace: Vec<DisplayId>,
    remove: Vec<DisplayId>,
}

fn plan_target_reconciliation(
    current: &[(DisplayTargetSignature, bool)],
    desired: &[DisplayTargetSignature],
) -> TargetReconciliationPlan {
    let desired_ids = desired
        .iter()
        .map(|signature| signature.display_id.clone())
        .collect::<HashSet<_>>();
    let mut plan = TargetReconciliationPlan::default();
    for next in desired {
        match current
            .iter()
            .find(|(existing, _)| existing.display_id == next.display_id)
        {
            Some((existing, true)) if existing == next => plan.keep.push(next.display_id.clone()),
            Some(_) => plan.replace.push(next.display_id.clone()),
            None => plan.start.push(next.display_id.clone()),
        }
    }
    for (existing, _) in current {
        if !desired_ids.contains(&existing.display_id) {
            plan.remove.push(existing.display_id.clone());
        }
    }
    plan
}

fn replace_after_success<T, E>(
    current: &mut T,
    build: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let replacement = build()?;
    Ok(std::mem::replace(current, replacement))
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

async fn start_video(
    sender: &PlaybackCommandSender,
    events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
    recovery: RecoveryFrameSender,
    assignment: ResolvedDisplayAssignment,
) -> Result<(), &'static str> {
    let wallpaper_id = assignment.profile.wallpaper_id();
    let source_path = assignment.wallpaper.source_path().to_path_buf();
    let source_text = source_path.to_string_lossy().into_owned();
    match sender
        .dispatch(Command::LoadVideo {
            wallpaper_id,
            path: source_text.clone(),
        })
        .await
    {
        ResponseResult::PlaybackState(_) => {}
        _ => return Err("video_load_request"),
    }
    let loaded = wait_for_video_load(events).await?;
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
    match sender
        .dispatch(Command::ApplyPlaybackSettings {
            settings,
            source_path: source_text.clone(),
        })
        .await
    {
        ResponseResult::PlaybackSettingsApplied => {}
        _ => return Err("playback_settings"),
    }
    if !matches!(
        sender.dispatch(Command::Play).await,
        ResponseResult::PlaybackState(_)
    ) {
        return Err("video_play");
    }

    let settings = EditorSettingsStore::for_current_user()
        .and_then(|store| store.load(wallpaper_id))
        .unwrap_or_else(|_| {
            WallpaperEditorSettings::default_for(wallpaper_id).expect("nonzero catalog id")
        });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let frame_sender = recovery.clone();
        let settings = settings.clone();
        let source_path = source_path.clone();
        match tokio::task::spawn_blocking(move || {
            frame_sender.apply_editor_settings(settings, source_path)
        })
        .await
        .unwrap_or(Err(ApplySettingsError::Renderer(
            wallpaper_renderer::RendererError::WindowUnavailable,
        ))) {
            Ok(()) => break,
            Err(ApplySettingsError::WallpaperNotActive) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(_) => return Err("editor_settings"),
        }
    }
    Ok(())
}

async fn wait_for_video_load(
    events: &mut tokio::sync::watch::Receiver<PlaybackStateEvent>,
) -> Result<PlaybackStateEvent, &'static str> {
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
}

fn apply_editor_settings(recovery: &RecoveryFrameSender, assignment: &ResolvedDisplayAssignment) {
    let wallpaper_id = assignment.profile.wallpaper_id();
    let settings = EditorSettingsStore::for_current_user()
        .and_then(|store| store.load(wallpaper_id))
        .unwrap_or_else(|_| {
            WallpaperEditorSettings::default_for(wallpaper_id).expect("nonzero catalog id")
        });
    if recovery
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

    fn primary_identity(wallpaper_id: u64, fps_override: Option<u16>) -> PrimaryAssignmentIdentity {
        PrimaryAssignmentIdentity {
            display_id: DisplayId::from_device_path("display-a").unwrap(),
            wallpaper_id,
            mode: DisplayPresentationMode::Independent,
            fps_override,
            source_path: PathBuf::from("C:/wallpapers/sample.mp4"),
        }
    }

    #[test]
    fn primary_assignment_identity_changes_only_for_profile_or_source_changes() {
        let current = primary_identity(11, Some(30));
        let same_assignment_after_geometry_change = primary_identity(11, Some(30));
        let changed_wallpaper = primary_identity(22, Some(30));
        let changed_fps = primary_identity(11, Some(60));

        assert_eq!(current, same_assignment_after_geometry_change);
        assert_ne!(current, changed_wallpaper);
        assert_ne!(current, changed_fps);
    }

    #[test]
    fn primary_assignment_identity_ignores_span_geometry_changes() {
        let display = display("display-a", 0, true);
        let source_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/pixels.png");
        let mut library = WallpaperLibrary::default();
        let wallpaper_id = library
            .add(source_path, "span-source.png", MediaKind::Image)
            .unwrap();
        let profile = DisplayProfile::new(
            display.id().clone(),
            wallpaper_id.get(),
            DisplayPresentationMode::Span,
            None,
        )
        .unwrap();
        let wallpaper = library.get(wallpaper_id).unwrap().clone();
        let target = display.bounds();
        let first = ResolvedDisplayAssignment {
            display: display.clone(),
            profile: profile.clone(),
            wallpaper: wallpaper.clone(),
            span_bounds: Some(
                SpanBounds::new(DisplayRect::new(0, 0, 3840, 1080).unwrap(), target).unwrap(),
            ),
        };
        let changed_viewport = ResolvedDisplayAssignment {
            span_bounds: Some(
                SpanBounds::new(DisplayRect::new(0, 0, 5760, 1080).unwrap(), target).unwrap(),
            ),
            ..first.clone()
        };

        assert_eq!(
            PrimaryAssignmentIdentity::from(&first),
            PrimaryAssignmentIdentity::from(&changed_viewport)
        );
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

    fn target_signature(
        id: &str,
        left: i32,
        dpi_scale_percent: u32,
        wallpaper_id: u64,
        fps_override: Option<u16>,
        source_path: &str,
    ) -> DisplayTargetSignature {
        DisplayTargetSignature {
            display_id: DisplayId::from_device_path(id).unwrap(),
            device_name: format!("\\\\.\\DISPLAY{}", id.chars().last().unwrap_or('1')),
            bounds: DisplayRect::new(left, 0, left + 1920, 1080).unwrap(),
            dpi_scale_percent,
            refresh_rate: Some(RefreshRate::new(60, 1).unwrap()),
            span_bounds: None,
            wallpaper_id,
            presentation_mode: DisplayPresentationMode::Independent,
            fps_override,
            source_path: PathBuf::from(source_path),
        }
    }

    #[test]
    fn topology_reconcile_keeps_unchanged_target_and_scopes_replacements() {
        let unchanged = target_signature("display-a", 0, 100, 10, Some(30), "a.png");
        let old_geometry = target_signature("display-b", 1920, 100, 20, Some(30), "b.png");
        let old_assignment = target_signature("display-c", 3840, 100, 30, None, "c.png");
        let disconnected = target_signature("display-d", 5760, 100, 40, None, "d.png");
        let changed_geometry = target_signature("display-b", 1920, 150, 20, Some(30), "b.png");
        let changed_assignment = target_signature("display-c", 3840, 100, 31, Some(60), "c2.png");
        let newly_connected = target_signature("display-e", 7680, 100, 50, Some(24), "e.png");

        let current = vec![
            (unchanged.clone(), true),
            (old_geometry, true),
            (old_assignment, true),
            (disconnected, true),
        ];
        let desired = vec![
            unchanged.clone(),
            changed_geometry,
            changed_assignment,
            newly_connected,
        ];
        let plan = plan_target_reconciliation(&current, &desired);

        assert_eq!(plan.keep, vec![unchanged.display_id]);
        assert_eq!(
            plan.start,
            vec![DisplayId::from_device_path("display-e").unwrap()]
        );
        assert_eq!(
            plan.replace,
            vec![
                DisplayId::from_device_path("display-b").unwrap(),
                DisplayId::from_device_path("display-c").unwrap(),
            ]
        );
        assert_eq!(
            plan.remove,
            vec![DisplayId::from_device_path("display-d").unwrap()]
        );
    }

    #[test]
    fn topology_reconcile_replaces_inactive_target_and_readds_disconnected_identity() {
        let disconnected = target_signature("display-a", 0, 100, 10, None, "a.png");
        let plan = plan_target_reconciliation(&[], std::slice::from_ref(&disconnected));
        assert_eq!(plan.start, vec![disconnected.display_id.clone()]);

        let removed = plan_target_reconciliation(&[(disconnected.clone(), true)], &[]);
        assert_eq!(removed.remove, vec![disconnected.display_id.clone()]);

        let returned = plan_target_reconciliation(&[], std::slice::from_ref(&disconnected));
        assert_eq!(returned.start, vec![disconnected.display_id]);

        let inactive = target_signature("display-b", 1920, 100, 20, None, "b.png");
        let retry = plan_target_reconciliation(
            &[(inactive.clone(), false)],
            std::slice::from_ref(&inactive),
        );
        assert_eq!(retry.replace, vec![inactive.display_id]);
    }

    #[test]
    fn failed_target_replacement_keeps_last_good_target() {
        let mut active = String::from("last-good-target");
        let result = replace_after_success(&mut active, || Err::<String, _>("attach failed"));
        assert_eq!(result, Err("attach failed"));
        assert_eq!(active, "last-good-target");

        let old = replace_after_success(&mut active, || {
            Ok::<_, &'static str>(String::from("replacement"))
        })
        .unwrap();
        assert_eq!(old, "last-good-target");
        assert_eq!(active, "replacement");
    }

    #[test]
    fn display_target_manager_handles_queued_topology_events_and_stops() {
        let mut manager = DisplayTargetManager::start(&[], None);
        manager.explorer_exited();
        manager.explorer_restarted();
        manager.reconcile(&ResolvedDisplayAssignments {
            primary_bounds: None,
            profiles: Vec::new(),
        });
        assert!(manager.stop());
    }
}
