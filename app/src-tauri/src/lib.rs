//! The window: it forwards the page's commands to `mohona-appcore`, keeps a
//! tray icon, closes to the tray when asked to, and shows a message when a
//! download finishes or fails (the window may be hidden, so this is done here
//! and not in the page).

use std::time::Duration;

use mohona_appcore::{
    AppError, Category, Config, ConfigChange, Engine, EngineStatus, ExtensionSettings, Plan,
    Details, PlanWindow, Queue, QueueChange, Row, Rule, SiteProfile, SpeedTracker, ToolInfo, TorrentList, Watcher,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
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
async fn add_download(
    app: State<'_, App>,
    url: String,
    folder: Option<String>,
    queue: Option<i64>,
    files: Option<Vec<u32>>,
) -> Result<i64> {
    match files {
        Some(files) => {
            app.engine
                .add_torrent(&url, folder.as_deref(), queue, files)
                .await
        }
        None => app.engine.add_to(&url, folder.as_deref(), queue).await,
    }
}

#[tauri::command]
async fn torrent_files(app: State<'_, App>, url: String) -> Result<TorrentList> {
    app.engine.torrent_files(&url).await
}

#[tauri::command]
async fn get_details(app: State<'_, App>, id: i64) -> Result<Details> {
    app.engine.details(id).await
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

#[tauri::command]
async fn get_plan(app: State<'_, App>) -> Result<Plan> {
    app.engine.plan().await
}

#[tauri::command]
async fn set_plan(app: State<'_, App>, windows: Vec<PlanWindow>) -> Result<Plan> {
    app.engine.set_plan(windows).await
}

#[tauri::command]
async fn set_after_action(
    app: State<'_, App>,
    action: Option<String>,
    countdown_seconds: Option<u32>,
) -> Result<Plan> {
    app.engine.set_after_action(action, countdown_seconds).await
}

#[tauri::command]
async fn cancel_action(app: State<'_, App>) -> Result<Plan> {
    app.engine.cancel_action().await
}

/// The text on the clipboard, if any (for "offer to download links I copy").
#[tauri::command]
async fn read_clipboard(handle: AppHandle) -> Result<Option<String>> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    // An empty clipboard, or one holding an image, is simply "nothing".
    Ok(handle.clipboard().read_text().ok().filter(|t| !t.is_empty()))
}

/// Asks the person for a folder (the system's own dialog); `None` when cancelled.
#[tauri::command]
async fn pick_folder(handle: AppHandle) -> Result<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx
        .await
        .map_err(|_| AppError::Engine("the folder dialog was closed".into()))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn get_categories(app: State<'_, App>) -> Result<Vec<Category>> {
    app.engine.categories().await
}

#[tauri::command]
async fn add_category(
    app: State<'_, App>,
    name: String,
    extensions: Vec<String>,
    folder: Option<String>,
) -> Result<Vec<Category>> {
    app.engine.add_category(&name, extensions, folder).await
}

#[tauri::command]
async fn remove_category(app: State<'_, App>, id: i64) -> Result<Vec<Category>> {
    app.engine.remove_category(id).await
}

#[tauri::command]
async fn get_tools(app: State<'_, App>) -> Result<Vec<ToolInfo>> {
    app.engine.tools().await
}

#[tauri::command]
async fn set_tool(app: State<'_, App>, name: String, path: Option<String>) -> Result<Vec<ToolInfo>> {
    app.engine.set_tool(name, path).await
}

#[tauri::command]
async fn get_video_browser(app: State<'_, App>) -> Result<Option<String>> {
    app.engine.video_browser().await
}

#[tauri::command]
async fn set_video_browser(app: State<'_, App>, browser: Option<String>) -> Result<Option<String>> {
    app.engine.set_video_browser(browser).await
}

/// Opens one of the tools' download pages in the browser; no other address is ever opened.
#[tauri::command]
async fn open_tool_page(handle: AppHandle, url: String) -> Result<()> {
    if !mohona_appcore::is_tool_page(&url) {
        return Err(AppError::Input("that address is not one of the tools' download pages".into()));
    }
    handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Engine(e.to_string()))
}

#[tauri::command]
async fn get_profiles(app: State<'_, App>) -> Result<Vec<SiteProfile>> {
    app.engine.profiles().await
}

#[tauri::command]
async fn set_profiles(
    app: State<'_, App>,
    profiles: Vec<SiteProfile>,
) -> Result<Vec<SiteProfile>> {
    app.engine.set_profiles(profiles).await
}

#[tauri::command]
async fn get_rules(app: State<'_, App>) -> Result<Vec<Rule>> {
    app.engine.rules().await
}

#[tauri::command]
async fn set_rules(app: State<'_, App>, rules: Vec<Rule>) -> Result<Vec<Rule>> {
    app.engine.set_rules(rules).await
}

#[tauri::command]
async fn get_queues(app: State<'_, App>) -> Result<Vec<Queue>> {
    app.engine.queues().await
}

#[tauri::command]
async fn add_queue(app: State<'_, App>, name: String) -> Result<Vec<Queue>> {
    app.engine.add_queue(&name).await
}

#[tauri::command]
async fn rename_queue(app: State<'_, App>, id: i64, name: String) -> Result<Vec<Queue>> {
    app.engine.rename_queue(id, &name).await
}

#[tauri::command]
async fn remove_queue(app: State<'_, App>, id: i64) -> Result<Vec<Queue>> {
    app.engine.remove_queue(id).await
}

#[tauri::command]
async fn set_queue(app: State<'_, App>, id: i64, change: QueueChange) -> Result<Vec<Queue>> {
    app.engine.set_queue(id, change).await
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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
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
            torrent_files,
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
            get_details,
            read_clipboard,
            get_tools,
            set_tool,
            open_tool_page,
            get_video_browser,
            set_video_browser,
            get_profiles,
            set_profiles,
            get_rules,
            set_rules,
            get_queues,
            add_queue,
            rename_queue,
            remove_queue,
            set_queue,
            get_plan,
            set_plan,
            set_after_action,
            cancel_action,
            pick_folder,
            get_categories,
            add_category,
            remove_category,
        ])
        .run(tauri::generate_context!())
        .expect("the Mohona window could not start");
}
