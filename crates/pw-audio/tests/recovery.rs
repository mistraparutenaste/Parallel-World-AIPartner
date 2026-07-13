use pw_audio::recovery::{AudioStreamFailure, CaptureAdapter, RecoveryController, RecoveryOutcome};
use pw_audio::recovery::{RecoveryEvent, RecoveryWorker};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Fake {
    devices: Vec<String>,
    attempts: Vec<Option<String>>,
    fail: bool,
}
impl CaptureAdapter for Fake {
    type Stream = ();
    fn devices(&mut self) -> Result<Vec<String>, AudioStreamFailure> {
        Ok(self.devices.clone())
    }
    fn open(&mut self, id: Option<&str>) -> Result<Self::Stream, AudioStreamFailure> {
        self.attempts.push(id.map(str::to_owned));
        if self.fail {
            Err(AudioStreamFailure::Busy)
        } else {
            Ok(())
        }
    }
}

#[test]
fn unavailable_reenumerates_and_prefers_the_selected_logical_device() {
    let fake = Fake {
        devices: vec!["mic-b".into(), "mic-a".into()],
        ..Fake::default()
    };
    let mut recovery = RecoveryController::new(fake, Some("mic-a".into()), 7);
    assert_eq!(
        recovery.recover(7, AudioStreamFailure::NotAvailable),
        RecoveryOutcome::Recovered { fallback: false }
    );
    assert_eq!(recovery.adapter().attempts, vec![Some("mic-a".into())]);
}

#[test]
fn unavailable_falls_back_to_default_when_selection_disappeared() {
    let fake = Fake {
        devices: vec!["mic-b".into()],
        ..Fake::default()
    };
    let mut recovery = RecoveryController::new(fake, Some("mic-a".into()), 2);
    assert_eq!(
        recovery.recover(2, AudioStreamFailure::NotAvailable),
        RecoveryOutcome::Recovered { fallback: true }
    );
    assert_eq!(recovery.adapter().attempts, vec![None]);
}

#[test]
fn explicit_stop_and_stale_generation_never_recover() {
    let mut recovery = RecoveryController::new(Fake::default(), None, 3);
    recovery.stop();
    assert_eq!(
        recovery.recover(3, AudioStreamFailure::DeviceChanged),
        RecoveryOutcome::Stopped
    );
    assert_eq!(
        recovery.recover(2, AudioStreamFailure::Busy),
        RecoveryOutcome::Stale
    );
    assert!(recovery.adapter().attempts.is_empty());
}

#[test]
fn callback_notification_is_bounded_and_counts_drops_and_depth() {
    let (sender, receiver, metrics) = pw_audio::recovery::failure_channel(1);
    assert!(sender.notify(AudioStreamFailure::Busy));
    assert!(!sender.notify(AudioStreamFailure::HostUnavailable));
    assert_eq!(metrics.depth(), 1);
    assert_eq!(metrics.dropped(), 1);
    assert_eq!(receiver.try_recv(), Some(AudioStreamFailure::Busy));
    assert_eq!(metrics.depth(), 0);
}

#[derive(Default)]
struct SharedFake {
    state: Arc<Mutex<SharedState>>,
}

#[derive(Default)]
struct SharedState {
    devices: Vec<String>,
    attempts: Vec<Option<String>>,
    live_streams: usize,
}

struct FakeStream(Arc<Mutex<SharedState>>);
impl Drop for FakeStream {
    fn drop(&mut self) {
        self.0.lock().unwrap().live_streams -= 1;
    }
}
impl CaptureAdapter for SharedFake {
    type Stream = FakeStream;
    fn devices(&mut self) -> Result<Vec<String>, AudioStreamFailure> {
        Ok(self.state.lock().unwrap().devices.clone())
    }
    fn open(&mut self, id: Option<&str>) -> Result<Self::Stream, AudioStreamFailure> {
        let mut state = self.state.lock().unwrap();
        state.attempts.push(id.map(str::to_owned));
        state.live_streams += 1;
        drop(state);
        Ok(FakeStream(Arc::clone(&self.state)))
    }
}

#[test]
fn worker_recovers_falls_back_emits_once_and_joins_on_stop() {
    let state = Arc::new(Mutex::new(SharedState {
        devices: vec!["mic-b".into()],
        ..SharedState::default()
    }));
    let (failures, receiver, _) = pw_audio::recovery::failure_channel(2);
    let events = Arc::new(Mutex::new(Vec::new()));
    let event_copy = Arc::clone(&events);
    let worker = RecoveryWorker::spawn(
        SharedFake {
            state: Arc::clone(&state),
        },
        Some("mic-a".into()),
        9,
        receiver,
        move |event| event_copy.lock().unwrap().push(event),
    );
    assert!(failures.notify(AudioStreamFailure::NotAvailable));
    for _ in 0..50 {
        if !events.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            RecoveryEvent::Recovering {
                failure: AudioStreamFailure::NotAvailable,
                attempt: 0
            },
            RecoveryEvent::Recovered { fallback: true },
        ]
    );
    assert_eq!(state.lock().unwrap().attempts, vec![None]);
    worker.stop_and_join();
    assert_eq!(state.lock().unwrap().live_streams, 0);
}

#[test]
fn worker_ignores_stale_failures_and_explicit_stop_never_restarts() {
    let state = Arc::new(Mutex::new(SharedState::default()));
    let (failures, receiver, _) = pw_audio::recovery::failure_channel(2);
    let worker = RecoveryWorker::spawn(
        SharedFake {
            state: Arc::clone(&state),
        },
        None,
        4,
        receiver,
        |_| {},
    );
    worker.set_generation(5);
    assert!(failures.notify(AudioStreamFailure::DeviceChanged));
    std::thread::sleep(Duration::from_millis(10));
    worker.stop_and_join();
    assert!(state.lock().unwrap().attempts.is_empty());
}
