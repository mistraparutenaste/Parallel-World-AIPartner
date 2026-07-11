pub mod definitions;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub fn ensure_windows<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    for definition in definitions::WINDOWS {
        if app.get_webview_window(definition.label).is_some() {
            continue;
        }

        WebviewWindowBuilder::new(
            app,
            definition.label,
            WebviewUrl::App(definition.url.into()),
        )
        .title(definition.title)
        .transparent(definition.transparent)
        .decorations(definition.decorations)
        .build()?;
    }

    Ok(())
}
