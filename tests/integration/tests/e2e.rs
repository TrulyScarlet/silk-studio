//! Mock end-to-end pipeline: controller commands drive scripted capture,
//! a mock encoder, and a mock muxer through the real engine, replay
//! buffer, and state machine (spec §27.4 subset; hardware paths land in
//! S2+).

use std::sync::Arc;
use std::time::{Duration, Instant};

use app_controller::{
    ClipAttribution, CollectingNotificationSink, Command, Controller, ControllerEvent,
    ControllerSettings, NotificationSeverity, NotificationSink,
};
use recorder_engine::RecorderState;
use test_support::{MockMuxer, MockVideoEncoder, ScriptedVideoCapture, TempDir, VideoStep};

fn make_controller(
    steps: Vec<VideoStep>,
    dir: &TempDir,
) -> (Controller, Arc<CollectingNotificationSink>) {
    let sink_impl = Arc::new(CollectingNotificationSink::new());
    let output_dir = dir.path().to_path_buf();
    let sink_dyn: Arc<dyn NotificationSink> = sink_impl.clone();
    let controller = Controller::with_settings(
        Box::new(move || {
            (
                Box::new(ScriptedVideoCapture::new(steps.clone()))
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        }),
        sink_dyn,
        ControllerSettings {
            output_dir,
            ..ControllerSettings::default()
        },
    );
    (controller, sink_impl)
}

/// Pump the controller until the buffer is primed (Ready), collecting all
/// events emitted along the way. Fails after 5 s.
fn pump_until_ready(controller: &mut Controller, collected: &mut Vec<ControllerEvent>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        collected.extend(controller.poll());
        if controller.state() == RecorderState::Ready && controller.buffered_packet_count() > 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "buffer never became ready; state={:?}, packets={}",
        controller.state(),
        controller.buffered_packet_count()
    );
}

fn pump_until_save_complete(controller: &mut Controller, collected: &mut Vec<ControllerEvent>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if collected.iter().any(|event| {
            matches!(
                event,
                ControllerEvent::ClipSaved { .. } | ControllerEvent::SaveFailed { .. }
            )
        }) {
            return;
        }
        collected.extend(controller.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("save worker did not complete: {collected:?}");
}

fn statuses(events: &[ControllerEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::StatusChanged { state } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

const FRAMES: u32 = 90;

#[test]
fn full_lifecycle_start_save_stop_produces_clip() {
    let dir = TempDir::new("e2e-lifecycle").expect("temp");
    let (mut controller, sink) = make_controller(vec![VideoStep::Frames(FRAMES)], &dir);

    // Save before start is rejected with a structured event.
    let early = controller.execute(&Command::SaveReplay);
    assert!(
        matches!(&early[0], ControllerEvent::CommandRejected { command, .. } if command == "save_replay")
    );

    let start_events = controller.execute(&Command::StartCapture);
    assert!(
        statuses(&start_events)
            .iter()
            .any(|state| state == "Buffering"),
        "start must enter Buffering before priming: {:?}",
        statuses(&start_events)
    );

    let mut collected = Vec::new();
    pump_until_ready(&mut controller, &mut collected);
    let mut trail = statuses(&start_events);
    trail.extend(statuses(&collected));
    assert!(trail.contains(&"Buffering".to_string()), "{trail:?}");
    assert_eq!(trail.last().map(String::as_str), Some("Ready"), "{trail:?}");

    let mut save_events = controller.execute(&Command::SaveReplay);
    pump_until_save_complete(&mut controller, &mut save_events);
    let saved = save_events
        .iter()
        .find_map(|e| match e {
            ControllerEvent::ClipSaved {
                path,
                duration_ms,
                size_bytes,
            } => Some((path.clone(), *duration_ms, *size_bytes)),
            _ => None,
        })
        .expect("clip_saved event expected");

    assert!(saved.1 > 0, "duration must be positive");
    assert!(saved.2 > 0, "size must be positive");

    let clip_path = std::path::PathBuf::from(&saved.0);
    assert!(clip_path.exists(), "published file must exist");
    let bytes = std::fs::read(&clip_path).expect("read clip");
    assert!(
        bytes.starts_with(b"SILKMOCKV1"),
        "mock container header expected"
    );
    assert!(
        !clip_path.to_string_lossy().ends_with(".part"),
        "no staging name may leak"
    );

    // Notifications flowed to the sink (success toast path).
    assert!(
        sink.snapshot().iter().any(|n| n.title == "Replay saved"),
        "expected success notification"
    );

    let stop_events = controller.execute(&Command::StopCapture);
    assert_eq!(
        statuses(&stop_events).last().map(String::as_str),
        Some("Stopped")
    );
    let save_metrics = controller.save_metrics();
    assert_eq!(save_metrics.saves_queued, 1);
    assert_eq!(save_metrics.saves_completed, 1);
    assert_eq!(save_metrics.saves_failed, 0);

    // Second stop rejected: nothing running.
    let again = controller.execute(&Command::StopCapture);
    assert!(matches!(&again[0], ControllerEvent::CommandRejected { .. }));

    // A stopped controller can create a fresh backend session and restart.
    let restart = controller.execute(&Command::StartCapture);
    assert!(restart.iter().any(
        |event| matches!(event, ControllerEvent::StatusChanged { state } if state == "Buffering")
    ));
    let mut restart_progress = Vec::new();
    pump_until_ready(&mut controller, &mut restart_progress);
    assert_eq!(controller.state(), RecorderState::Ready);
    controller.execute(&Command::StopCapture);
}

#[test]
fn attributed_save_publishes_under_a_game_directory() {
    let dir = TempDir::new("e2e-attributed-save").expect("temp");
    let (mut controller, _) = make_controller(vec![VideoStep::Frames(FRAMES)], &dir);
    controller
        .update_settings(ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            organize_by_game: true,
            ..ControllerSettings::default()
        })
        .expect("enable game organization");

    controller.execute(&Command::StartCapture);
    let mut progress = Vec::new();
    pump_until_ready(&mut controller, &mut progress);

    let attribution = ClipAttribution {
        game_name: Some("Example Game".to_string()),
    };
    let mut save_events =
        controller.execute_with_attribution(&Command::SaveReplay, Some(&attribution));
    pump_until_save_complete(&mut controller, &mut save_events);
    let saved_path = save_events
        .iter()
        .find_map(|event| match event {
            ControllerEvent::ClipSaved { path, .. } => Some(std::path::PathBuf::from(path)),
            _ => None,
        })
        .expect("clip_saved event expected");

    assert_eq!(
        saved_path.parent().and_then(|path| path.file_name()),
        Some("Example Game".as_ref())
    );
    assert!(saved_path.exists());
    controller.execute(&Command::StopCapture);
}

#[test]
fn duplicate_start_is_rejected_while_running() {
    let dir = TempDir::new("e2e-dup").expect("temp");
    let (mut controller, _) = make_controller(vec![VideoStep::Frames(30)], &dir);
    controller.execute(&Command::StartCapture);
    let second = controller.execute(&Command::StartCapture);
    assert!(
        matches!(&second[0], ControllerEvent::CommandRejected { command, .. } if command == "start_capture")
    );
    controller.execute(&Command::StopCapture);
}

#[test]
fn mux_failure_surfaces_save_failed_and_capture_continues() {
    let dir = TempDir::new("e2e-muxfail").expect("temp");
    let sink_impl = Arc::new(CollectingNotificationSink::new());
    let output_dir = dir.path().to_path_buf();
    let steps = vec![VideoStep::Frames(FRAMES)];
    let sink_dyn: Arc<dyn NotificationSink> = sink_impl.clone();
    let mut controller = Controller::with_settings(
        Box::new(move || {
            let mut muxer = MockMuxer::default();
            muxer.fail_on_call = Some(0);
            (
                Box::new(ScriptedVideoCapture::new(steps.clone()))
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(muxer) as Box<dyn muxer::Muxer>,
            )
        }),
        sink_dyn,
        ControllerSettings {
            output_dir,
            ..ControllerSettings::default()
        },
    );

    controller.execute(&Command::StartCapture);
    let mut collected = Vec::new();
    pump_until_ready(&mut controller, &mut collected);

    let mut failed = controller.execute(&Command::SaveReplay);
    pump_until_save_complete(&mut controller, &mut failed);
    assert!(
        failed.iter().any(|e| matches!(e, ControllerEvent::SaveFailed { code, .. } if code == "MUXER_WRITE_FAILED")),
        "mux failure must surface as save_failed: {failed:?}"
    );
    assert!(
        sink_impl
            .snapshot()
            .iter()
            .any(|n| n.severity == NotificationSeverity::Error),
        "failure notification required"
    );

    // Capture continues after a failed save (BUF-007): the buffer still
    // holds a primed replay window.
    assert!(controller.buffered_packet_count() > 0);

    // A later save succeeds once the injected failure window passed.
    let mut retry = controller.execute(&Command::SaveReplay);
    pump_until_save_complete(&mut controller, &mut retry);
    assert!(
        retry
            .iter()
            .any(|e| matches!(e, ControllerEvent::ClipSaved { .. })),
        "retry after failure should succeed: {retry:?}"
    );
}

#[test]
fn configuration_defaults_round_trip_through_disk() {
    use configuration::{load, save, AppConfig};

    let dir = TempDir::new("e2e-config").expect("temp");
    let path = dir.path().join("config.json");

    let loaded = load(&path).expect("missing file yields defaults");
    assert!(!loaded.warnings.is_empty());
    assert_eq!(loaded.settings.replay.duration_seconds, 60);

    let custom = AppConfig {
        replay: configuration::ReplaySettings {
            duration_seconds: 120,
        },
        ..AppConfig::default()
    };
    save(&path, &custom).expect("save");
    let reloaded = load(&path).expect("reload");
    assert_eq!(reloaded.settings.replay.duration_seconds, 120);
    assert_eq!(reloaded.file_version, configuration::CURRENT_CONFIG_VERSION);
}
