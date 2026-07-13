//! Adapter over the official Tauri updater. A checked `Update` is kept intact
//! between signature-verifying download and installation.

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use super::service::{
    CheckedUpdate, IdempotentFlusher, InstallDisposition, UpdateBackend, UpdateError,
};

pub struct SettingsUpdateEmitter<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> super::service::UpdateEmitter for SettingsUpdateEmitter<R> {
    fn emit_to_settings(&self, state: &pw_contracts::UpdateStateDto) {
        if let Some(window) = self.0.get_webview_window("settings")
            && let Err(error) = window.emit(super::service::UPDATE_PROGRESS_EVENT, state)
        {
            tracing::warn!(%error, "failed to emit updater progress to settings");
        }
    }
}

pub struct TauriUpdateBackend<R: Runtime> {
    app: AppHandle<R>,
    flusher: IdempotentFlusher,
}

impl<R: Runtime> TauriUpdateBackend<R> {
    pub fn new(app: AppHandle<R>, flusher: IdempotentFlusher) -> Self {
        Self { app, flusher }
    }
}

struct TauriCheckedUpdate {
    update: Update,
}

#[async_trait]
impl CheckedUpdate for TauriCheckedUpdate {
    fn version(&self) -> &str {
        &self.update.version
    }
    fn notes(&self) -> Option<&str> {
        self.update.body.as_deref()
    }
    async fn download(
        &mut self,
        progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<Vec<u8>, UpdateError> {
        self.update
            .download(|chunk, total| progress(chunk as u64, total), || {})
            .await
            .map_err(|error| UpdateError::Backend(error.to_string()))
    }
    async fn install(self: Box<Self>, bytes: Vec<u8>) -> Result<InstallDisposition, UpdateError> {
        self.update
            .install(bytes)
            .map_err(|error| UpdateError::Backend(error.to_string()))?;
        #[cfg(target_os = "windows")]
        return Ok(InstallDisposition::PluginOwnsProcessExit);
        #[cfg(not(target_os = "windows"))]
        Ok(InstallDisposition::InstalledNeedsRelaunch)
    }
}

#[async_trait]
impl<R: Runtime> UpdateBackend for TauriUpdateBackend<R> {
    async fn check(&self) -> Result<Option<Box<dyn CheckedUpdate>>, UpdateError> {
        let flusher = self.flusher.clone();
        let app = self.app.clone();
        let updater = self
            .app
            .updater_builder()
            .on_before_exit(move || {
                flusher.flush();
                app.cleanup_before_exit();
            })
            .build()
            .map_err(|error| UpdateError::Backend(error.to_string()))?;
        updater
            .check()
            .await
            .map(|update| {
                update
                    .map(|update| Box::new(TauriCheckedUpdate { update }) as Box<dyn CheckedUpdate>)
            })
            .map_err(|error| UpdateError::Backend(error.to_string()))
    }
}
