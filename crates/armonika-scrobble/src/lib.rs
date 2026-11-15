//! Scrobbling: ListenBrainz and a persistent outbox.
//!
//! A play the user submits is written to `scrobble_outbox`, one row per linked service,
//! in the same transaction that counts the play, so it is never lost. The outbox worker
//! sends the due rows in batches and retries with a growing delay. "Now playing" is
//! sent once, best effort.
//!
//! Account tokens are stored sealed with the server secret key and never logged.

pub mod accounts;
pub mod listenbrainz;
pub mod outbox;

use std::sync::Arc;

use armonika_core::Result;
use armonika_core::db::{Connection, Db};
use armonika_core::ids::{TrackId, UserId, resolve_track, resolve_user};
use armonika_core::secret::SecretBox;
use rusqlite::{OptionalExtension, params};

/// User-Agent of every request to an external service.
pub fn user_agent() -> String {
    format!(
        "Armonika/{} ( https://github.com/filocorsa/Armonika )",
        armonika_core::VERSION
    )
}

/// A play as the services want it: what the tags of the track say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen {
    /// When the play started, in Unix seconds. `None` for "now playing".
    pub listened_at: Option<i64>,
    pub title: String,
    /// The artist as credited on the track.
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub track_number: Option<i64>,
    /// MusicBrainz recording id, if the file has one.
    pub recording_mbid: Option<String>,
}

/// Why a service did not take a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// Network error, rate limit or server error: try again later.
    Retry,
    /// The service refused the credentials: keep the data, the user must link again.
    Auth,
    /// The service refused the data itself: sending it again will not help.
    Rejected,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SubmitError::Retry => "temporary failure",
            SubmitError::Auth => "credentials refused",
            SubmitError::Rejected => "data refused",
        })
    }
}

impl std::error::Error for SubmitError {}

/// An external scrobbling service. The calls block: run them with `spawn_blocking`.
pub trait ScrobbleService: Send + Sync {
    /// The name stored in `scrobble_accounts.service` and `scrobble_outbox.service`.
    fn name(&self) -> &'static str;

    /// Where the service is, for log messages. Never holds a credential.
    fn endpoint(&self) -> String {
        self.name().to_owned()
    }

    /// The most listens one `submit` call takes.
    fn max_batch(&self) -> usize {
        100
    }

    /// Tells the service what the account is playing now.
    fn now_playing(&self, token: &str, listen: &Listen) -> std::result::Result<(), SubmitError>;

    /// Submits finished plays (at most [`ScrobbleService::max_batch`]).
    fn submit(&self, token: &str, batch: &[Listen]) -> std::result::Result<(), SubmitError>;
}

/// The tags of the track with internal key `track`, as a [`Listen`] without a time. `None`
/// if the track is gone or has no title or artist (the services need both).
pub(crate) fn load_listen(conn: &Connection, track: i64) -> Result<Option<Listen>> {
    Ok(conn
        .prepare_cached(
            "SELECT title, artist_credit, album_title, duration_ms, track_no, mb_recording_id
             FROM tracks WHERE id = ?1",
        )?
        .query_row([track], |r| {
            let title: Option<String> = r.get(0)?;
            let artist: Option<String> = r.get(1)?;
            Ok(match (title, artist) {
                (Some(title), Some(artist)) if !title.is_empty() && !artist.is_empty() => {
                    Some(Listen {
                        listened_at: None,
                        title,
                        artist,
                        album: r.get(2)?,
                        duration_ms: r.get(3)?,
                        track_number: r.get(4)?,
                        recording_mbid: r.get(5)?,
                    })
                }
                _ => None,
            })
        })
        .optional()?
        .flatten())
}

struct Delivery {
    db: Db,
    secrets: SecretBox,
    services: Vec<Arc<dyn ScrobbleService>>,
}

/// Where plays are reported. Cheap to clone. The default sink has no services: it
/// still records submitted plays for the accounts that are linked, and sends no
/// "now playing".
#[derive(Clone, Default)]
pub struct ScrobbleSink {
    delivery: Option<Arc<Delivery>>,
}

impl std::fmt::Debug for ScrobbleSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScrobbleSink(..)")
    }
}

impl ScrobbleSink {
    /// A sink that sends "now playing" through `services`.
    pub fn new(db: Db, secrets: SecretBox, services: Vec<Arc<dyn ScrobbleService>>) -> Self {
        ScrobbleSink {
            delivery: Some(Arc::new(Delivery {
                db,
                secrets,
                services,
            })),
        }
    }

    /// Queues a play of `track` by `user` started at `played_at_ms` for every service
    /// the user has linked. Call it on the connection and inside the transaction that
    /// counts the play, so both are written or neither is.
    pub fn submitted(
        &self,
        conn: &Connection,
        user: &UserId,
        track: &TrackId,
        played_at_ms: i64,
    ) -> Result<()> {
        let Some(user) = resolve_user(conn, user)? else {
            return Ok(());
        };
        let Some(track) = resolve_track(conn, track)? else {
            return Ok(());
        };
        let services: Vec<String> = conn
            .prepare_cached("SELECT service FROM scrobble_accounts WHERE user_id = ?1")?
            .query_map([user], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for service in services {
            conn.prepare_cached(
                "INSERT INTO scrobble_outbox (user_id, service, track_id, played_at, next_attempt_at)
                 VALUES (?1, ?2, ?3, ?4, 0)",
            )?
            .execute(params![user, service, track, played_at_ms])?;
        }
        Ok(())
    }

    /// Tells every linked service that `user` is playing `track`. Best effort: a
    /// failure is not retried and not reported.
    pub async fn now_playing(&self, user: &UserId, track: &TrackId) {
        let Some(delivery) = self.delivery.clone() else {
            return;
        };
        if delivery.services.is_empty() {
            return;
        }
        let (user, track) = (user.clone(), track.clone());
        let secrets = delivery.secrets.clone();
        let found = delivery
            .db
            .read(move |c| {
                let (Some(user), Some(track)) =
                    (resolve_user(c, &user)?, resolve_track(c, &track)?)
                else {
                    return Ok(None);
                };
                let Some(listen) = load_listen(c, track)? else {
                    return Ok(None);
                };
                Ok(Some((accounts::tokens_of(c, &secrets, user)?, listen)))
            })
            .await;
        let Ok(Some((tokens, listen))) = found else {
            return;
        };
        for (service, token) in tokens {
            let Some(svc) = delivery
                .services
                .iter()
                .find(|s| s.name() == service)
                .cloned()
            else {
                continue;
            };
            let listen = listen.clone();
            let sent = tokio::task::spawn_blocking(move || svc.now_playing(&token, &listen)).await;
            if !matches!(sent, Ok(Ok(()))) {
                tracing::debug!(service, "could not send now playing");
            }
        }
    }
}
