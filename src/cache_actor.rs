#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::{
    fs,
    path::PathBuf,
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::value::RawValue;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tracing::{debug, error, info, info_span, warn};

use crate::config::Configuration;

const DB_SCHEMA: &str = "create table if not exists pending_submissions
(
    id integer primary key,
    submission text not null
)";

#[derive(Debug)]
pub struct CacheActor(Option<(UnboundedSender<CacheAction>, JoinHandle<()>)>);

impl CacheActor {
    pub fn start(config: &Configuration) -> Result<CacheActor> {
        if !config.enable_cache {
            debug!("submission cache is disabled");
            return Ok(CacheActor(None));
        }

        let db = open_cache_file(config)?;
        let (tx, rx) = mpsc::unbounded_channel();

        let handle = thread::spawn(move || run(rx, db));

        Ok(CacheActor(Some((tx, handle))))
    }

    pub fn shutdown(self) {
        if let Some((channel, handle)) = self.0 {
            drop(channel);
            handle.join().unwrap();
        }
    }

    pub fn cache_submissions(&self, submissions: Vec<Box<RawValue>>) {
        if let Some((tx, _)) = &self.0 {
            tx.send(CacheAction::CacheFailedSubmissions(submissions))
                .map_err(|_| ())
                .expect("Cache actor is gone");
        }
    }

    pub async fn load_pending_submissions(&self) -> Vec<Box<RawValue>> {
        let Some((tx, _)) = &self.0 else {
            return Vec::new();
        };

        let (responder_tx, responder_rx) = oneshot::channel();
        tx.send(CacheAction::GetCachedSubmissions(responder_tx))
            .map_err(|_| ())
            .expect("Cache actor is gone");

        responder_rx.await.expect("Cache actor did not respond")
    }
}

fn open_cache_file(config: &Configuration) -> Result<Connection> {
    let default_path = default_submission_cache_path();
    let legacy_path = legacy_default_submission_cache_path();

    let mut cache_file = config.cache_file.as_deref().unwrap_or(&default_path);

    debug!(?cache_file, "opening submission cache");

    // Ensure the cache directory exists so that the database file can be created if
    // necessary
    if let Some(cache_file_dir) = cache_file.parent() {
        debug!(
            ?cache_file_dir,
            "creating submission cache file directories"
        );
        let mut builder = fs::DirBuilder::new();

        #[cfg(unix)]
        builder.mode(0o700);

        builder
            .recursive(true)
            .create(cache_file_dir)
            .with_context(|| {
                format!(
                    "Failed to create submission cache directory at {}",
                    cache_file_dir.display()
                )
            })?;
    }

    // If the cache file location is not explicitly set and the cache file exists at
    // the old location but not at the new default location, attempt to move it
    if config.cache_file.is_none() && !cache_file.is_file() && legacy_path.is_file() {
        debug!(old = ?legacy_path, new = ?cache_file, "attempting to move submission cache file");
        match fs::rename(&legacy_path, cache_file) {
            Ok(()) => {
                info!(old = ?legacy_path, new = ?cache_file, "migrated submission cache to new location");
            }
            Err(e) => {
                // Stick to the old location if the migration fails
                warn!(
                    old = ?legacy_path,
                    new = ?cache_file,
                    error = ?e,
                    "failed to move submission cache to new location, sticking to old location"
                );
                cache_file = &legacy_path;
            }
        }
    }

    Connection::open(cache_file).with_context(|| {
        format!(
            "Failed to open submission cache file at {}",
            cache_file.display()
        )
    })
}

/// Returns the default location of the submission cache.
fn default_submission_cache_path() -> PathBuf {
    let mut p = dirs::data_local_dir().expect("No state/cache directory");
    p.push(env!("CARGO_PKG_NAME"));
    p.push("submission-cache.sqlite3");
    p
}

/// Returns the old default path of the submission cache.
fn legacy_default_submission_cache_path() -> PathBuf {
    let mut base = dirs::data_local_dir().expect("No state/cache directory");
    base.push("listenbrainz-mpd-cache.sqlite3");
    base
}

fn run(mut receiver: UnboundedReceiver<CacheAction>, mut db: Connection) {
    let _span = info_span!("cache").entered();

    db.execute(DB_SCHEMA, ())
        .expect("Failed to initialize database");

    while let Some(action) = receiver.blocking_recv() {
        match action {
            CacheAction::CacheFailedSubmissions(submissions) => {
                debug!(count = submissions.len(), "caching submissions");
                cache_submissions(&mut db, &submissions).expect("Failed to cache submissions");
            }
            CacheAction::GetCachedSubmissions(responder) => {
                let pending =
                    load_pending_submissions(&mut db).expect("Failed to load pending submissions");
                debug!(count = pending.len(), "loaded pending submissions");
                if let Err(pending) = responder.send(pending) {
                    error!("pending response was not received, inserting values back");
                    cache_submissions(&mut db, &pending)
                        .expect("Failed to re-insert pending submissions");
                }
            }
        }
    }
}

fn cache_submissions(db: &mut Connection, submissions: &[Box<RawValue>]) -> Result<()> {
    let tx = db.transaction()?;

    {
        let mut stmt =
            tx.prepare_cached("insert into pending_submissions (submission) values (?)")?;

        for submission in submissions {
            stmt.execute([submission.get()])?;
        }
    }

    tx.commit()?;
    Ok(())
}

fn load_pending_submissions(db: &mut Connection) -> Result<Vec<Box<RawValue>>> {
    let tx = db.transaction()?;
    let mut out = Vec::new();

    {
        let mut select_stmt =
            tx.prepare_cached("select id, submission from pending_submissions limit 99")?;
        let mut res = select_stmt.query(())?;
        let mut ids = Vec::new();

        while let Some(row) = res.next()? {
            let id = row.get::<_, i64>(0)?;
            ids.push(id);

            let value = row.get::<_, String>(1)?;
            let value = RawValue::from_string(value).unwrap();
            out.push(value);
        }

        // Delete rows we just selected
        let mut delete_stmt = tx.prepare_cached("delete from pending_submissions where id = ?")?;

        for id in ids {
            delete_stmt.execute((id,))?;
        }
    }

    tx.commit()?;
    Ok(out)
}

enum CacheAction {
    CacheFailedSubmissions(Vec<Box<RawValue>>),
    GetCachedSubmissions(oneshot::Sender<Vec<Box<RawValue>>>),
}
