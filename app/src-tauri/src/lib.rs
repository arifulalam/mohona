//! The window: it only forwards the page's commands to `mohona-appcore`.

use mohona_appcore::{AppError, Engine, EngineStatus, Row, SpeedTracker};
use tauri::State;
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

pub fn run() {
    tauri::Builder::default()
        .manage(App {
            engine: Engine::standard(),
            tracker: Mutex::new(SpeedTracker::new()),
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
        ])
        .run(tauri::generate_context!())
        .expect("the Mohona window could not start");
}
