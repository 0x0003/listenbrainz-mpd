#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::{
    fs,
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::value::RawValue;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tracing::{debug, error, info_span};

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

        let cache_file = &config.cache_file;
        debug!(?cache_file, "opening submission cache");

        // Ensure the cache directory exists so that the database file can be created if
        // necessary
        if let Some(cache_file_dir) = cache_file.parent() {
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

        let db = Connection::open(cache_file).with_context(|| {
            format!(
                "Failed to open submission cache file at {}",
                cache_file.display()
            )
        })?;

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
