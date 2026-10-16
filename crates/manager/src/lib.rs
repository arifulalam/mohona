//! Mohona download manager.
//!
//! One long-lived task owns every download. Front ends hold a cheap,
//! cloneable [`Manager`] handle, send it commands (add, pause, resume,
//! cancel, remove) and watch [`ManagerEvent`]s. Every state change is saved
//! in the database first, so a restart picks up where it stopped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_core::{CancelToken, DownloadRequest, Error as CoreError, Event, Outcome};
use mohona_net::HttpClient;
use mohona_store::{DownloadRow, Status, Store, StoreError};
use tokio::sync::{broadcast, mpsc, oneshot};

pub use mohona_store::{DownloadRow as Download, Status as DownloadStatus};

/// How often the downloaded byte count is written to the database.
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

/// Errors a command can return.
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    /// The database failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// No download has this id.
    #[error("no download with id {0}")]
    NotFound(i64),
    /// The command makes no sense in the download's current state.
    #[error("cannot do that to a download that is {0}")]
    WrongState(&'static str),
    /// The manager has shut down.
    #[error("the download manager has stopped")]
    Stopped,
}

/// Something a front end may want to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagerEvent {
    /// A download was added.
    Added {
        /// Its id.
        id: i64,
    },
    /// A download changed state.
    State {
        /// Its id.
        id: i64,
        /// The new state.
        status: Status,
    },
    /// Bytes received so far.
    Progress {
        /// Its id.
        id: i64,
        /// Bytes on disk.
        downloaded: u64,
        /// Size, if known.
        total: Option<u64>,
    },
    /// A download failed.
    Error {
        /// Its id.
        id: i64,
        /// What went wrong.
        message: String,
    },
    /// A download was removed.
    Removed {
        /// Its id.
        id: i64,
    },
}

/// What a running download's task has been asked to do when it stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Pause,
    Cancel,
    Shutdown,
}

enum Command {
    Add {
        url: String,
        dest_dir: PathBuf,
        queue_id: Option<i64>,
        reply: oneshot::Sender<Result<i64, ManagerError>>,
    },
    Pause(i64, Reply),
    Resume(i64, Reply),
    Cancel(i64, Reply),
    Remove {
        id: i64,
        delete_file: bool,
        reply: Reply,
    },
    List(oneshot::Sender<Result<Vec<DownloadRow>, ManagerError>>),
    SetQueueLimit(i64, u32, Reply),
    Shutdown(oneshot::Sender<()>),
    /// Sent by a download's own task when it ends.
    Ended {
        id: i64,
        result: Result<Outcome, CoreError>,
    },
    /// Sent by a download's own task when it learned about the file.
    Started {
        id: i64,
        url: String,
        filename: String,
        total: Option<u64>,
        resumed: u64,
    },
    Saved {
        id: i64,
        downloaded: u64,
    },
}

type Reply = oneshot::Sender<Result<(), ManagerError>>;

/// Handle to the download manager. Clone it freely.
#[derive(Clone)]
pub struct Manager {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
}

/// Settings for [`Manager::start`].
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    /// Tune each download's connections adaptively (the default). Tests
    /// turn this off.
    pub adaptive: bool,
    /// The most connections one download may use.
    pub connections: usize,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            adaptive: true,
            connections: mohona_core::DEFAULT_CONNECTIONS,
        }
    }
}

impl Manager {
    /// Start the manager task. Downloads left running by an earlier crash are
    /// put back in their queues and started again (they resume where they
    /// stopped).
    pub fn start(
        store: Store,
        client: HttpClient,
        config: ManagerConfig,
    ) -> Result<Manager, ManagerError> {
        store.reset_interrupted()?;
        let (commands, inbox) = mpsc::channel(256);
        let (events, _) = broadcast::channel(1024);
        let actor = Actor {
            store,
            client,
            config,
            inbox,
            commands: commands.clone(),
            events: events.clone(),
            running: HashMap::new(),
            stopping: false,
        };
        tokio::spawn(actor.run());
        Ok(Manager { commands, events })
    }

    /// Watch what the manager does. Slow receivers may miss events; the
    /// database always has the truth.
    pub fn subscribe(&self) -> broadcast::Receiver<ManagerEvent> {
        self.events.subscribe()
    }

    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, ManagerError>>) -> Command,
    ) -> Result<T, ManagerError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(make(tx))
            .await
            .map_err(|_| ManagerError::Stopped)?;
        rx.await.map_err(|_| ManagerError::Stopped)?
    }

    /// Add a download. It starts as soon as its queue has a free slot.
    pub async fn add(
        &self,
        url: impl Into<String>,
        dest_dir: impl Into<PathBuf>,
    ) -> Result<i64, ManagerError> {
        let (url, dest_dir) = (url.into(), dest_dir.into());
        self.ask(|reply| Command::Add {
            url,
            dest_dir,
            queue_id: None,
            reply,
        })
        .await
    }

    /// Stop a download but keep what was downloaded.
    pub async fn pause(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Pause(id, r)).await
    }

    /// Put a paused, failed or canceled download back in its queue.
    pub async fn resume(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Resume(id, r)).await
    }

    /// Stop a download and delete its partial data.
    pub async fn cancel(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Cancel(id, r)).await
    }

    /// Remove a download from the list. A running one is cancelled first.
    /// With `delete_file`, a finished file is deleted from disk too.
    pub async fn remove(&self, id: i64, delete_file: bool) -> Result<(), ManagerError> {
        self.ask(|reply| Command::Remove {
            id,
            delete_file,
            reply,
        })
        .await
    }

    /// Every download, oldest first.
    pub async fn list(&self) -> Result<Vec<DownloadRow>, ManagerError> {
        self.ask(Command::List).await
    }

    /// How many downloads the queue may run at once.
    pub async fn set_queue_limit(
        &self,
        queue_id: i64,
        max_active: u32,
    ) -> Result<(), ManagerError> {
        self.ask(|r| Command::SetQueueLimit(queue_id, max_active, r))
            .await
    }

    /// Stop everything cleanly, keeping partial data, and end the manager.
    /// Downloads that were running are marked queued again, so they resume
    /// on the next start.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.commands.send(Command::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

struct RunningDownload {
    url: String,
    dest_dir: PathBuf,
    cancel: CancelToken,
    intent: Option<Intent>,
}

struct Actor {
    store: Store,
    client: HttpClient,
    config: ManagerConfig,
    inbox: mpsc::Receiver<Command>,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    running: HashMap<i64, RunningDownload>,
    /// Set once shutdown starts: nothing new may begin.
    stopping: bool,
}

impl Actor {
    async fn run(mut self) {
        self.pump();
        let mut shutdown: Option<oneshot::Sender<()>> = None;
        while let Some(command) = self.inbox.recv().await {
            if let Command::Shutdown(done) = command {
                self.stopping = true;
                for running in self.running.values_mut() {
                    running.intent = Some(Intent::Shutdown);
                    running.cancel.cancel();
                }
                shutdown = Some(done);
            } else {
                self.handle(command);
            }
            if shutdown.is_some() && self.running.is_empty() {
                break;
            }
        }
        if let Some(done) = shutdown {
            let _ = done.send(());
        }
    }

    fn emit(&self, event: ManagerEvent) {
        let _ = self.events.send(event);
    }

    fn set_status(&self, id: i64, status: Status, error: Option<&str>) {
        if self.store.set_status(id, status, error).is_ok() {
            self.emit(ManagerEvent::State { id, status });
        }
        if let Some(message) = error {
            self.emit(ManagerEvent::Error {
                id,
                message: message.to_string(),
            });
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Add {
                url,
                dest_dir,
                queue_id,
                reply,
            } => {
                let result = self
                    .store
                    .add_download(url.trim(), &dest_dir.to_string_lossy(), queue_id)
                    .map_err(ManagerError::from);
                if let Ok(id) = result {
                    self.emit(ManagerEvent::Added { id });
                    self.pump();
                }
                let _ = reply.send(result);
            }
            Command::Pause(id, reply) => {
                let _ = reply.send(self.stop(id, Intent::Pause));
            }
            Command::Cancel(id, reply) => {
                let _ = reply.send(self.stop(id, Intent::Cancel));
            }
            Command::Resume(id, reply) => {
                let result = self.requeue(id);
                if result.is_ok() {
                    self.pump();
                }
                let _ = reply.send(result);
            }
            Command::Remove {
                id,
                delete_file,
                reply,
            } => {
                let _ = reply.send(self.remove(id, delete_file));
            }
            Command::List(reply) => {
                let _ = reply.send(self.store.list_downloads().map_err(Into::into));
            }
            Command::SetQueueLimit(queue_id, max_active, reply) => {
                let result = self
                    .store
                    .set_queue_max_active(queue_id, max_active)
                    .map_err(Into::into);
                self.pump();
                let _ = reply.send(result);
            }
            Command::Shutdown(_) => unreachable!("handled in run"),
            Command::Started {
                id,
                url,
                filename,
                total,
                resumed,
            } => {
                if self.running.contains_key(&id) {
                    let _ = self.store.set_started(id, &url, &filename, total, resumed);
                    self.set_status(id, Status::Downloading, None);
                }
            }
            Command::Saved { id, downloaded } => {
                let _ = self.store.set_downloaded(id, downloaded);
            }
            Command::Ended { id, result } => self.ended(id, result),
        }
    }

    fn row(&self, id: i64) -> Result<DownloadRow, ManagerError> {
        self.store.download(id)?.ok_or(ManagerError::NotFound(id))
    }

    /// Pause or cancel. A running download is told to stop and settles when
    /// its task reports back; one that is not running settles right away.
    fn stop(&mut self, id: i64, intent: Intent) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        if let Some(running) = self.running.get_mut(&id) {
            running.intent = Some(intent);
            running.cancel.cancel();
            return Ok(());
        }
        match (row.status, intent) {
            (Status::Queued, Intent::Pause) => {
                self.set_status(id, Status::Paused, None);
                Ok(())
            }
            (Status::Queued | Status::Paused | Status::Failed, Intent::Cancel) => {
                self.discard(&row);
                self.set_status(id, Status::Canceled, None);
                Ok(())
            }
            (status, _) => Err(ManagerError::WrongState(status.as_str())),
        }
    }

    fn requeue(&mut self, id: i64) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        match row.status {
            Status::Paused | Status::Failed | Status::Canceled => {
                self.set_status(id, Status::Queued, None);
                Ok(())
            }
            status => Err(ManagerError::WrongState(status.as_str())),
        }
    }

    fn remove(&mut self, id: i64, delete_file: bool) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        if let Some(running) = self.running.get_mut(&id) {
            // Cancel, and let `ended` finish the job.
            running.intent = Some(Intent::Cancel);
            running.cancel.cancel();
        } else {
            self.discard(&row);
        }
        if delete_file {
            if let Some(path) = &row.path {
                let _ = std::fs::remove_file(path);
            }
        }
        self.store.delete_download(id)?;
        self.emit(ManagerEvent::Removed { id });
        self.pump();
        Ok(())
    }

    fn discard(&self, row: &DownloadRow) {
        if row.status != Status::Completed {
            mohona_core::discard_partial(std::path::Path::new(&row.dest_dir), &row.url);
        }
    }

    fn ended(&mut self, id: i64, result: Result<Outcome, CoreError>) {
        let running = self.running.remove(&id);
        let intent = running.as_ref().and_then(|r| r.intent);
        // The row is gone when the download was removed while it ran: only
        // its partial data is left to clean up.
        let Ok(Some(row)) = self.store.download(id) else {
            if let (Some(running), Some(Intent::Cancel)) = (running, intent) {
                mohona_core::discard_partial(&running.dest_dir, &running.url);
            }
            self.pump();
            return;
        };
        match (result, intent) {
            (Ok(outcome), _) => {
                let _ =
                    self.store
                        .set_completed(id, &outcome.path.to_string_lossy(), outcome.bytes);
                self.emit(ManagerEvent::State {
                    id,
                    status: Status::Completed,
                });
            }
            (Err(_), Some(Intent::Pause)) => self.set_status(id, Status::Paused, None),
            (Err(_), Some(Intent::Shutdown)) => {
                // Not a state the user chose: run again at the next start.
                let _ = self.store.set_status(id, Status::Queued, None);
            }
            (Err(_), Some(Intent::Cancel)) => {
                self.discard(&row);
                self.set_status(id, Status::Canceled, None);
            }
            (Err(error), None) => {
                let message = describe(&error);
                self.set_status(id, Status::Failed, Some(&message));
            }
        }
        self.pump();
    }

    /// Start whatever the queues have room for.
    fn pump(&mut self) {
        if self.stopping {
            return;
        }
        let Ok(ids) = self.store.next_to_start() else {
            return;
        };
        for id in ids {
            if self.running.contains_key(&id) {
                continue;
            }
            let Ok(row) = self.row(id) else { continue };
            self.start(row);
        }
    }

    fn start(&mut self, row: DownloadRow) {
        let cancel = CancelToken::new();
        self.running.insert(
            row.id,
            RunningDownload {
                url: row.url.clone(),
                dest_dir: PathBuf::from(&row.dest_dir),
                cancel: cancel.clone(),
                intent: None,
            },
        );
        self.set_status(row.id, Status::Probing, None);
        let mut request = DownloadRequest::new(row.url.clone(), row.dest_dir.clone());
        request.connections = self.config.connections;
        request.retry.adaptive = self.config.adaptive;
        request.cancel = cancel;
        tokio::spawn(run_download(
            Arc::new(self.client.clone()),
            request,
            row.id,
            self.commands.clone(),
            self.events.clone(),
        ));
    }
}

async fn run_download(
    client: Arc<HttpClient>,
    request: DownloadRequest,
    id: i64,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
) {
    let (tx, mut rx) = mpsc::channel::<Event>(64);
    let engine = {
        let client = client.clone();
        let request = request.clone();
        async move { mohona_core::download(&client, &request, Some(&tx)).await }
    };
    tokio::pin!(engine);
    let mut last_save = Instant::now();
    let mut latest = 0u64;
    let result = loop {
        tokio::select! {
            result = &mut engine => break result,
            Some(event) = rx.recv() => match event {
                Event::Started { url, filename, total, resumed, .. } => {
                    latest = resumed;
                    let _ = commands
                        .send(Command::Started { id, url, filename, total, resumed })
                        .await;
                }
                Event::Progress { downloaded, total } => {
                    latest = downloaded;
                    let _ = events.send(ManagerEvent::Progress { id, downloaded, total });
                    if last_save.elapsed() >= SAVE_INTERVAL {
                        last_save = Instant::now();
                        let _ = commands.send(Command::Saved { id, downloaded }).await;
                    }
                }
                _ => {}
            },
        }
    };
    if result.is_err() && latest > 0 {
        let _ = commands
            .send(Command::Saved {
                id,
                downloaded: latest,
            })
            .await;
    }
    let _ = commands.send(Command::Ended { id, result }).await;
}

/// An error with its causes, as one line.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    text
}
