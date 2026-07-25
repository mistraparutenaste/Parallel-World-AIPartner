//! Single-flight updater state machine.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pw_contracts::{SCHEMA_VERSION, UpdateStateDto, UpdateStatusDto};
use thiserror::Error;

pub const UPDATE_PROGRESS_EVENT: &str = pw_contracts::UPDATE_PROGRESS_EVENT;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("{0}")]
    Backend(String),
    #[error("another update operation is already running")]
    Busy,
    #[error("the approved update is no longer available")]
    ApprovalMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallDisposition {
    PluginOwnsProcessExit,
    InstalledNeedsRelaunch,
}

#[async_trait]
pub trait CheckedUpdate: Send {
    fn version(&self) -> &str;
    fn notes(&self) -> Option<&str>;
    async fn download(
        &mut self,
        progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<Vec<u8>, UpdateError>;
    async fn install(self: Box<Self>, bytes: Vec<u8>) -> Result<InstallDisposition, UpdateError>;
}

#[async_trait]
pub trait UpdateBackend: Send + Sync {
    async fn check(&self) -> Result<Option<Box<dyn CheckedUpdate>>, UpdateError>;
}

pub trait UpdateEmitter: Send + Sync {
    fn emit_to_settings(&self, state: &UpdateStateDto);
}

#[derive(Default)]
struct NoopEmitter;

impl UpdateEmitter for NoopEmitter {
    fn emit_to_settings(&self, _state: &UpdateStateDto) {}
}

/// Runs the durable pre-exit flush once even when service and plugin hooks both invoke it.
#[derive(Clone)]
pub struct IdempotentFlusher {
    flushed: Arc<std::sync::atomic::AtomicBool>,
    flush: Arc<dyn Fn() + Send + Sync>,
}

impl IdempotentFlusher {
    pub fn new(flush: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            flushed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            flush: Arc::new(flush),
        }
    }

    pub fn flush(&self) {
        if !self.flushed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            (self.flush)();
        }
    }
}

struct ServiceState {
    snapshot: UpdateStateDto,
    pending: Option<Box<dyn CheckedUpdate>>,
    operation_in_flight: bool,
}

pub struct UpdateService {
    backend: Option<Arc<dyn UpdateBackend>>,
    emitter: Arc<dyn UpdateEmitter>,
    flusher: IdempotentFlusher,
    state: Mutex<ServiceState>,
}

impl UpdateService {
    pub fn disabled(current_version: impl Into<String>) -> Self {
        Self::new_internal(
            current_version.into(),
            None,
            Arc::new(NoopEmitter),
            IdempotentFlusher::new(|| {}),
        )
    }

    pub fn enabled(
        current_version: impl Into<String>,
        backend: Arc<dyn UpdateBackend>,
        emitter: Arc<dyn UpdateEmitter>,
        flusher: IdempotentFlusher,
    ) -> Self {
        Self::new_internal(current_version.into(), Some(backend), emitter, flusher)
    }

    fn new_internal(
        current_version: String,
        backend: Option<Arc<dyn UpdateBackend>>,
        emitter: Arc<dyn UpdateEmitter>,
        flusher: IdempotentFlusher,
    ) -> Self {
        let status = if backend.is_some() {
            UpdateStatusDto::UpToDate
        } else {
            UpdateStatusDto::Disabled
        };
        Self {
            backend,
            emitter,
            flusher,
            state: Mutex::new(ServiceState {
                snapshot: UpdateStateDto {
                    schema_version: SCHEMA_VERSION,
                    status,
                    current_version,
                    available_version: None,
                    notes: None,
                    error: None,
                },
                pending: None,
                operation_in_flight: false,
            }),
        }
    }

    pub fn snapshot(&self) -> UpdateStateDto {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot
            .clone()
    }

    fn publish(&self, state: &mut ServiceState, status: UpdateStatusDto) {
        state.snapshot.status = status;
        self.emitter.emit_to_settings(&state.snapshot);
    }

    /// Checks the configured signed endpoint with single-flight exclusion.
    ///
    /// # Errors
    /// Returns disabled, busy, network, metadata, or signature backend errors.
    pub async fn check(&self) -> Result<UpdateStateDto, UpdateError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| UpdateError::Backend("updater is disabled".into()))?
            .clone();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.operation_in_flight {
                return Err(UpdateError::Busy);
            }
            state.operation_in_flight = true;
            state.snapshot.error = None;
            self.publish(&mut state, UpdateStatusDto::Checking);
        }
        let checked = backend.check().await;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.operation_in_flight = false;
        match checked {
            Ok(Some(update)) => {
                state.snapshot.available_version = Some(update.version().to_owned());
                state.snapshot.notes = update.notes().map(ToOwned::to_owned);
                state.pending = Some(update);
                self.publish(&mut state, UpdateStatusDto::Available);
                Ok(state.snapshot.clone())
            }
            Ok(None) => {
                state.pending = None;
                state.snapshot.available_version = None;
                state.snapshot.notes = None;
                self.publish(&mut state, UpdateStatusDto::UpToDate);
                Ok(state.snapshot.clone())
            }
            Err(error) => {
                state.snapshot.error = Some(error.to_string());
                self.publish(&mut state, UpdateStatusDto::Failed);
                Err(error)
            }
        }
    }

    /// Downloads and installs the exact checked update approved by the user.
    ///
    /// # Errors
    /// Returns busy, stale-approval, download, signature, or installation errors.
    pub async fn install(&self, approved_version: &str) -> Result<InstallDisposition, UpdateError> {
        let mut update = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.operation_in_flight {
                return Err(UpdateError::Busy);
            }
            if state.snapshot.available_version.as_deref() != Some(approved_version) {
                return Err(UpdateError::ApprovalMismatch);
            }
            let update = state.pending.take().ok_or(UpdateError::ApprovalMismatch)?;
            state.operation_in_flight = true;
            state.snapshot.error = None;
            self.publish(&mut state, UpdateStatusDto::Downloading);
            update
        };
        let emitter = self.emitter.clone();
        let progress_snapshot = self.snapshot();
        let bytes = update
            .download(Box::new(move |_chunk, _total| {
                emitter.emit_to_settings(&progress_snapshot);
            }))
            .await;
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => return self.fail_install(error),
        };
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.publish(&mut state, UpdateStatusDto::Installing);
        }
        self.flusher.flush();
        let disposition = match update.install(bytes).await {
            Ok(value) => value,
            Err(error) => return self.fail_install(error),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.operation_in_flight = false;
        self.publish(&mut state, UpdateStatusDto::RestartPending);
        Ok(disposition)
    }

    fn fail_install<T>(&self, error: UpdateError) -> Result<T, UpdateError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.operation_in_flight = false;
        state.snapshot.error = Some(error.to_string());
        self.publish(&mut state, UpdateStatusDto::Failed);
        Err(error)
    }

    pub fn flush_before_exit(&self) {
        self.flusher.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct FakeUpdate {
        version: String,
        installed: Arc<AtomicUsize>,
        disposition: InstallDisposition,
    }
    #[async_trait]
    impl CheckedUpdate for FakeUpdate {
        fn version(&self) -> &str {
            &self.version
        }
        fn notes(&self) -> Option<&str> {
            Some("notes")
        }
        async fn download(
            &mut self,
            progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
        ) -> Result<Vec<u8>, UpdateError> {
            progress(3, Some(3));
            Ok(vec![1, 2, 3])
        }
        async fn install(
            self: Box<Self>,
            bytes: Vec<u8>,
        ) -> Result<InstallDisposition, UpdateError> {
            assert_eq!(bytes, [1, 2, 3]);
            self.installed.fetch_add(1, Ordering::SeqCst);
            Ok(self.disposition)
        }
    }
    struct FakeBackend {
        checks: AtomicUsize,
        update: Mutex<Option<Box<dyn CheckedUpdate>>>,
    }
    #[async_trait]
    impl UpdateBackend for FakeBackend {
        async fn check(&self) -> Result<Option<Box<dyn CheckedUpdate>>, UpdateError> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            Ok(self.update.lock().unwrap().take())
        }
    }
    #[derive(Default)]
    struct RecordingEmitter(Mutex<Vec<UpdateStateDto>>);
    impl UpdateEmitter for RecordingEmitter {
        fn emit_to_settings(&self, state: &UpdateStateDto) {
            self.0.lock().unwrap().push(state.clone());
        }
    }

    fn service(
        disposition: InstallDisposition,
    ) -> (
        UpdateService,
        Arc<RecordingEmitter>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    ) {
        let installed = Arc::new(AtomicUsize::new(0));
        let flushed = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(FakeBackend {
            checks: AtomicUsize::new(0),
            update: Mutex::new(Some(Box::new(FakeUpdate {
                version: "2.0.0".into(),
                installed: installed.clone(),
                disposition,
            }))),
        });
        let emitter = Arc::new(RecordingEmitter::default());
        let flush_count = flushed.clone();
        (
            UpdateService::enabled(
                "1.0.0",
                backend,
                emitter.clone(),
                IdempotentFlusher::new(move || {
                    flush_count.fetch_add(1, Ordering::SeqCst);
                }),
            ),
            emitter,
            installed,
            flushed,
        )
    }

    #[test]
    fn check_then_explicit_approval_installs_same_update_and_flushes_once() {
        tauri::async_runtime::block_on(async {
            let (service, emitter, installed, flushed) =
                service(InstallDisposition::PluginOwnsProcessExit);
            assert_eq!(
                service.check().await.unwrap().status,
                UpdateStatusDto::Available
            );
            assert!(matches!(
                service.install("1.9.0").await,
                Err(UpdateError::ApprovalMismatch)
            ));
            assert_eq!(
                service.install("2.0.0").await.unwrap(),
                InstallDisposition::PluginOwnsProcessExit
            );
            service.flush_before_exit();
            assert_eq!(installed.load(Ordering::SeqCst), 1);
            assert_eq!(flushed.load(Ordering::SeqCst), 1);
            let statuses: Vec<_> = emitter
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.status.clone())
                .collect();
            assert!(statuses.ends_with(&[
                UpdateStatusDto::Downloading,
                UpdateStatusDto::Downloading,
                UpdateStatusDto::Installing,
                UpdateStatusDto::RestartPending
            ]));
        });
    }

    #[test]
    fn macos_disposition_requires_adapter_relaunch() {
        tauri::async_runtime::block_on(async {
            let (service, _, _, _) = service(InstallDisposition::InstalledNeedsRelaunch);
            service.check().await.unwrap();
            assert_eq!(
                service.install("2.0.0").await.unwrap(),
                InstallDisposition::InstalledNeedsRelaunch
            );
        });
    }
}
