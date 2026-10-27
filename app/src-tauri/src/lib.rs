//! The window: it forwards the page's commands to `mohona-appcore`, keeps a
//! tray icon, closes to the tray when asked to, and shows a message when a
//! download finishes or fails (the window may be hidden, so this is done here
//! and not in the page).

use std::time::Duration;

use mohona_appcore::{
    AppError, Config, ConfigChange, Engine, EngineStatus, ExtensionSettings, Row, SpeedTracker,
    Watcher,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

struct App {
    engine: Engine,
    tracker: Mutex<SpeedTracker>,
}

type Result<T> = std::result::Result<T, AppError>;

#[tauri::command]
async fn engine_status(app: State<'_, App>) -> Result<EngineStatus> {
    Ok(app.engine.status().await)
}

#[tauri::command]
async fn start_engine(app: State<'_, App>) -> Result<EngineStatus> {
    app.engine.start().await
}

#[tauri::command]
async fn list_downloads(app: State<'_, App>) -> Result<Vec<Row>> {
    let mut tracker = app.tracker.lock().await;
    app.engine.list(&mut tracker).await
}

#[tauri::command]
async fn add_download(app: State<'_, App>, url: String, folder: Option<String>) -> Result<i64> {
    app.engine.add(&url, folder.as_deref()).await
}

#[tauri::command]
async fn pause_download(app: State<'_, App>, id: i64) -> Result<()> {
    app.engine.pause(id).await
}

#[tauri::command]
async fn resume_download(app: State<'_, App>, id: i64) -> Result<()> {
    app.engine.resume(id).await
}

#[tauri::command]
async fn cancel_download(app: State<'_, App>, id: i64) -> Result<()> {
    app.engine.cancel(id).await
}

#[tauri::command]
async fn remove_download(app: State<'_, App>, id: i64, delete_file: bool) -> Result<()> {
    app.engine.remove(id, delete_file).await
}

/// Opens a finished file with its program. The path is the engine's, and
/// programs and scripts are refused (see `Engine::openable_file`).
#[tauri::command]
async fn open_download(handle: AppHandle, app: State<'_, App>, id: i64) -> Result<()> {
    let path = app.engine.openable_file(id).await?;
    handle
        .opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Engine(e.to_string()))
}

#[tauri::command]
async fn show_download(handle: AppHandle, app: State<'_, App>, id: i64) -> Result<()> {
    let path = app.engine.finished_file(id).await?;
    handle
        .opener()
        .reveal_item_in_dir(path)
        .map_err(|e| AppError::Engine(e.to_string()))
}

#[tauri::command]
async fn get_config(app: State<'_, App>) -> Result<Config> {
    app.engine.config().await
}

#[tauri::command]
async fn set_config(app: State<'_, App>, change: ConfigChange) -> Result<Config> {
    app.engine.set_config(change).await
}

#[tauri::command]
async fn get_extension_settings(app: State<'_, App>) -> Result<ExtensionSettings> {
    app.engine.extension_settings().await
}

#[tauri::command]
async fn set_extension_settings(
    app: State<'_, App>,
    settings: ExtensionSettings,
) -> Result<ExtensionSettings> {
    app.engine.set_extension_settings(settings).await
}

fn show_window(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(handle: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(handle, "show", "Open Mohona", true, None::<&str>)?;
    let quit = MenuItem::with_id(
        handle,
        "quit",
        "Quit (downloads keep going)",
        true,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(handle, "stop", "Quit and stop Mohona", true, None::<&str>)?;
    let menu = Menu::with_items(handle, &[&show, &quit, &stop])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("Mohona")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|handle, event| match event.id.as_ref() {
            "show" => show_window(handle),
            "quit" => handle.exit(0),
            "stop" => {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let app = handle.state::<App>();
                    let _ = app.engine.stop().await;
                    handle.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = handle.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(handle)?;
    Ok(())
}

/// Looks at the list every two seconds and shows a message for each download
/// that finished or failed (when the person has not switched messages off).
fn watch_downloads(handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let app = handle.state::<App>();
        let mut watcher = Watcher::new();
        let mut tracker = SpeedTracker::new();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            match app.engine.list(&mut tracker).await {
                Ok(rows) => {
                    let notices = watcher.observe(&rows);
                    if notices.is_empty() {
                        continue;
                    }
                    let wanted = app
                        .engine
                        .config()
                        .await
                        .map_or(true, |config| config.notifications);
                    if wanted {
                        for notice in notices {
                            let _ = handle
                                .notification()
                                .builder()
                                .title(notice.title)
                                .body(notice.body)
                                .show();
                        }
                    }
                }
                // The engine was away: what it lists next is a new starting point.
                Err(_) => watcher.reset(),
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(App {
            engine: Engine::standard(),
            tracker: Mutex::new(SpeedTracker::new()),
        })
        .setup(|app| {
            build_tray(app.handle())?;
            watch_downloads(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Decided here, not by the page: hide to the tray, or really quit.
                api.prevent_close();
                let handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let app = handle.state::<App>();
                    let keep = app.engine.config().await.map_or(true, |c| c.keep_in_tray);
                    if keep {
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    } else {
                        handle.exit(0);
                    }
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            start_engine,
            list_downloads,
            add_download,
            pause_download,
            resume_download,
            cancel_download,
            remove_download,
            open_download,
            show_download,
            get_config,
            set_config,
            get_extension_settings,
            set_extension_settings,
        ])
        .run(tauri::generate_context!())
        .expect("the Mohona window could not start");
}
