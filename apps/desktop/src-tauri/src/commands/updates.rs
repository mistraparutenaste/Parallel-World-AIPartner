//! Settings-only signed updater commands.

use pw_contracts::UpdateStateDto;
use tauri::{AppHandle, State};

use crate::updates::UpdateService;
use crate::updates::service::InstallDisposition;

#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction owns State.
pub fn get_update_state(state: State<'_, UpdateService>) -> UpdateStateDto {
    state.snapshot()
}

#[tauri::command]
/// Performs a signed update check.
///
/// # Errors
/// Returns a display-safe backend or single-flight error.
pub async fn check_for_updates(state: State<'_, UpdateService>) -> Result<UpdateStateDto, String> {
    state.check().await.map_err(|error| error.to_string())
}

trait RestartRequester {
    fn restart(self) -> !;
}

impl<R: tauri::Runtime> RestartRequester for AppHandle<R> {
    fn restart(self) -> ! {
        AppHandle::restart(&self)
    }
}

fn finish_install<R: RestartRequester>(disposition: InstallDisposition, restart: R) {
    if disposition == InstallDisposition::InstalledNeedsRelaunch {
        restart.restart();
    }
}

#[tauri::command]
/// Installs only the version explicitly approved by the Settings UI.
///
/// # Errors
/// Returns an error when approval is stale, another operation is active, or installation fails.
pub async fn install_update(
    approved_version: String,
    state: State<'_, UpdateService>,
    app: AppHandle,
) -> Result<(), String> {
    let disposition = state
        .install(&approved_version)
        .await
        .map_err(|error| error.to_string())?;
    finish_install(disposition, app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{InstallDisposition, RestartRequester, finish_install};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct FakeRestart(Arc<AtomicUsize>);
    impl RestartRequester for FakeRestart {
        fn restart(self) -> ! {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("restart requested")
        }
    }

    #[test]
    fn macos_install_disposition_requests_restart() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = std::panic::catch_unwind({
            let calls = calls.clone();
            move || {
                finish_install(
                    InstallDisposition::InstalledNeedsRelaunch,
                    FakeRestart(calls),
                );
            }
        });
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn plugin_owned_exit_does_not_request_restart() {
        let calls = Arc::new(AtomicUsize::new(0));
        finish_install(
            InstallDisposition::PluginOwnsProcessExit,
            FakeRestart(calls.clone()),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
