//! The outbox worker: sends the due rows of `scrobble_outbox` to the services.
//!
//! A row is deleted once its service took it. On a network error or a 5xx answer it
//! is tried again after 1 minute, then 2, 4 and so on up to 6 hours. When the service
//! refuses the token the rows stay and a warning is logged once per user; linking a
//! new token makes them due again. A play older than 14 days is dropped.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use armonika_core::Result;
use armonika_core::db::{Db, now_ms};
use armonika_core::secret::SecretBox;
use rusqlite::params;
use tokio::sync::watch;

use crate::{Listen, ScrobbleService, SubmitError, accounts, load_listen};

const MINUTE_MS: i64 = 60_000;
const MAX_WAIT_MS: i64 = 6 * 60 * MINUTE_MS;
const MAX_AGE_MS: i64 = 14 * 24 * 60 * MINUTE_MS;

/// How often `run` looks for due rows.
const POLL: Duration = Duration::from_secs(15);

/// Most rows read per pass; the rest wait for the next one.
const PASS_LIMIT: i64 = 5000;

#[derive(Debug)]
struct Row {
    id: i64,
    user: i64,
    service: String,
    track: i64,
    played_at: i64,
    attempts: i64,
}

/// Wait before the next try of a row that failed after `attempts` earlier tries.
fn backoff_ms(attempts: i64) -> i64 {
    let shift = u32::try_from(attempts.clamp(0, 20)).unwrap_or(20);
    (MINUTE_MS << shift).min(MAX_WAIT_MS)
}

/// Sends the rows of the outbox. Keep one for the life of the server: it remembers
/// which users were already warned about a refused token.
pub struct Worker {
    db: Db,
    secrets: SecretBox,
    services: Vec<Arc<dyn ScrobbleService>>,
    warned: HashSet<(i64, String)>,
}

impl Worker {
    pub fn new(db: Db, secrets: SecretBox, services: Vec<Arc<dyn ScrobbleService>>) -> Worker {
        Worker {
            db,
            secrets,
            services,
            warned: HashSet::new(),
        }
    }

    /// One pass as of `now` (Unix milliseconds): drops what is too old, then sends the
    /// rows whose time has come.
    pub async fn process_due(&mut self, now: i64) -> Result<()> {
        let rows = self
            .db
            .write(move |c| {
                c.execute(
                    "DELETE FROM scrobble_outbox WHERE played_at < ?1",
                    [now - MAX_AGE_MS],
                )?;
                let rows = c
                    .prepare(
                        "SELECT id, user_id, service, track_id, played_at, attempts
                         FROM scrobble_outbox WHERE next_attempt_at <= ?1
                         ORDER BY played_at, id LIMIT ?2",
                    )?
                    .query_map(params![now, PASS_LIMIT], |r| {
                        Ok(Row {
                            id: r.get(0)?,
                            user: r.get(1)?,
                            service: r.get(2)?,
                            track: r.get(3)?,
                            played_at: r.get(4)?,
                            attempts: r.get(5)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        let mut groups: BTreeMap<(i64, String), Vec<Row>> = BTreeMap::new();
        for row in rows {
            groups
                .entry((row.user, row.service.clone()))
                .or_default()
                .push(row);
        }
        for ((user, service), rows) in groups {
            // One group failing (a token that cannot be opened, a database error) must
            // not hold back the others; its rows stay and are tried again next pass.
            if let Err(e) = self.send_group(user, &service, rows, now).await {
                tracing::warn!(service, user, error = %e, "cannot send the plays of one account");
            }
        }
        Ok(())
    }

    async fn send_group(
        &mut self,
        user: i64,
        service: &str,
        rows: Vec<Row>,
        now: i64,
    ) -> Result<()> {
        let Some(svc) = self.services.iter().find(|s| s.name() == service).cloned() else {
            return Ok(());
        };
        let secrets = self.secrets.clone();
        let name = service.to_owned();
        let tracks: Vec<i64> = rows.iter().map(|r| r.track).collect();
        let (token, listens) = self
            .db
            .read(move |c| {
                let token = accounts::token_by_key(c, &secrets, user, &name)?;
                let listens = tracks
                    .into_iter()
                    .map(|t| load_listen(c, t))
                    .collect::<Result<Vec<_>>>()?;
                Ok((token, listens))
            })
            .await?;
        // A row without an account (it was unlinked) or without a title and artist to
        // send can never be delivered.
        let mut sendable: Vec<(Row, Listen)> = Vec::new();
        let mut hopeless = Vec::new();
        for (row, listen) in rows.into_iter().zip(listens) {
            match listen {
                Some(mut l) if token.is_some() => {
                    l.listened_at = Some(row.played_at.div_euclid(1000));
                    sendable.push((row, l));
                }
                _ => hopeless.push(row.id),
            }
        }
        if !hopeless.is_empty() {
            self.delete(hopeless).await?;
        }
        let Some(token) = token else {
            return Ok(());
        };
        // A refused batch is split in halves until the refused play is alone: one bad
        // listen must not take the good ones with it.
        let mut pending: std::collections::VecDeque<&[(Row, Listen)]> =
            sendable.chunks(svc.max_batch().max(1)).collect();
        // Plays refused on their own. They are dropped only if the service accepted
        // another play of this group in this pass: a refusal that hits every request
        // (wrong URL, a method or media type it does not take) says nothing about the
        // plays and must not drain the queue.
        let mut refused: Vec<&Row> = Vec::new();
        let mut accepted = false;
        while let Some(chunk) = pending.pop_front() {
            let batch: Vec<Listen> = chunk.iter().map(|(_, l)| l.clone()).collect();
            let (svc, token) = (svc.clone(), token.clone());
            let outcome = tokio::task::spawn_blocking(move || svc.submit(&token, &batch))
                .await
                .unwrap_or(Err(SubmitError::Retry));
            let ids: Vec<i64> = chunk.iter().map(|(r, _)| r.id).collect();
            match outcome {
                Ok(()) => {
                    accepted = true;
                    self.warned.remove(&(user, service.to_owned()));
                    self.delete(ids).await?;
                }
                Err(SubmitError::Rejected) if chunk.len() > 1 => {
                    let (first, second) = chunk.split_at(chunk.len() / 2);
                    pending.push_front(second);
                    pending.push_front(first);
                }
                Err(SubmitError::Rejected) => refused.push(&chunk[0].0),
                Err(e) => {
                    if e == SubmitError::Auth && self.warned.insert((user, service.to_owned())) {
                        tracing::warn!(
                            service,
                            user,
                            "the service refused the stored token; plays are kept until the \
                            user links the account again"
                        );
                    }
                    // This chunk and the rest of the group wait for the next try.
                    let mut failed: Vec<(i64, i64)> =
                        chunk.iter().map(|(r, _)| (r.id, r.attempts)).collect();
                    for rest in pending.drain(..) {
                        failed.extend(rest.iter().map(|(r, _)| (r.id, r.attempts)));
                    }
                    self.reschedule(failed, now).await?;
                    break;
                }
            }
        }
        if accepted {
            for row in &refused {
                tracing::warn!(
                    service,
                    user,
                    track = row.track,
                    played_at = row.played_at,
                    "the service refused a play; dropping it"
                );
            }
            self.delete(refused.iter().map(|r| r.id).collect()).await?;
        } else if !refused.is_empty() {
            tracing::warn!(
                service,
                endpoint = %svc.endpoint(),
                plays = refused.len(),
                "the service refused every play; keeping them and trying again later"
            );
            self.reschedule(refused.iter().map(|r| (r.id, r.attempts)).collect(), now)
                .await?;
        }
        Ok(())
    }

    async fn delete(&self, ids: Vec<i64>) -> Result<()> {
        self.db
            .write(move |c| {
                let tx = c.transaction()?;
                for id in ids {
                    tx.execute("DELETE FROM scrobble_outbox WHERE id = ?1", [id])?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn reschedule(&self, rows: Vec<(i64, i64)>, now: i64) -> Result<()> {
        self.db
            .write(move |c| {
                let tx = c.transaction()?;
                for (id, attempts) in rows {
                    tx.execute(
                        "UPDATE scrobble_outbox
                         SET attempts = attempts + 1, next_attempt_at = ?2 WHERE id = ?1",
                        params![id, now + backoff_ms(attempts)],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

/// Runs the worker until `stop` becomes true (or its sender is dropped).
pub async fn run(
    db: Db,
    secrets: SecretBox,
    services: Vec<Arc<dyn ScrobbleService>>,
    mut stop: watch::Receiver<bool>,
) {
    let mut worker = Worker::new(db, secrets, services);
    loop {
        if *stop.borrow() {
            break;
        }
        if let Err(e) = worker.process_due(now_ms()).await {
            tracing::warn!(error = %e, "the scrobble outbox pass failed");
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(POLL) => {}
        }
    }
}
