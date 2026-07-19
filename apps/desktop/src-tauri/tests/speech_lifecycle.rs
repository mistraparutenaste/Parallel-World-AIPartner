//! Speech pipeline lifecycle tests that need a mock Tauri app.
//!
//! These live in an integration test (not the lib unit tests) because
//! instantiating the Tauri runtime links dialog code that imports
//! `TaskDialogIndirect`, which only resolves under the Common-Controls
//! v6 manifest embedded into integration test binaries by `build.rs`.

use std::time::{Duration, Instant};

use parallel_world_desktop::speech::{SpeechService, SttModelPaths};
use pw_contracts::SttPhaseDto;
use tauri::Manager;

fn missing_model_paths() -> SttModelPaths {
    let root = std::env::temp_dir().join("pw-speech-missing-models");
    SttModelPaths {
        vad_model: root.join("vad/silero_vad.onnx"),
        recognizer_dir: root.join("stt"),
    }
}

fn wait_for_phase(service: &SpeechService, phase: &SttPhaseDto) -> SttPhaseDto {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if service.current_state().phase == *phase {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    service.current_state().phase
}

#[test]
fn start_during_shutdown_queues_and_completes_after_worker_exit() {
    let app = tauri::test::mock_app();
    app.manage(SpeechService::default());
    let handle = app.handle().clone();
    let service = handle.state::<SpeechService>();
    let release = service.testing_install_blocked_stopping_worker();

    // The old worker has not exited yet: the request must be queued
    // instead of failing with a transient error.
    service
        .start(handle.clone(), missing_model_paths(), None)
        .expect("start queues while the previous pipeline stops");
    assert_eq!(service.current_state().phase, SttPhaseDto::Starting);

    release.send(()).unwrap();
    // The deferred start launches a fresh pipeline, which then reports
    // the missing models as unavailable.
    assert_eq!(
        wait_for_phase(&service, &SttPhaseDto::Unavailable),
        SttPhaseDto::Unavailable
    );
}

#[test]
fn stop_discards_a_queued_start() {
    let app = tauri::test::mock_app();
    app.manage(SpeechService::default());
    let handle = app.handle().clone();
    let service = handle.state::<SpeechService>();
    let release = service.testing_install_blocked_stopping_worker();

    service
        .start(handle.clone(), missing_model_paths(), None)
        .expect("start queues while the previous pipeline stops");
    service.stop();
    release.send(()).unwrap();

    // The deferred thread reaps the old pipeline but must not start a
    // new one after the explicit stop.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !service.testing_has_running_entry() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!service.testing_has_running_entry());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(service.current_state().phase, SttPhaseDto::Stopped);
    assert!(!service.testing_has_pending_start());
}
