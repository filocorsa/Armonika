//! ListenBrainz: `POST /1/submit-listens` and `GET /1/validate-token`.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::{Listen, ScrobbleService, SubmitError, user_agent};

/// Name of the service in the database.
pub const NAME: &str = "listenbrainz";

/// ListenBrainz takes at most 1000 listens per request; 100 keeps requests small.
const MAX_BATCH: usize = 100;

/// A ListenBrainz server.
#[derive(Clone)]
pub struct ListenBrainz {
    base: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for ListenBrainz {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenBrainz")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl ListenBrainz {
    /// A client for the server at `base_url`, for example `https://api.listenbrainz.org`.
    pub fn new(base_url: &str) -> ListenBrainz {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(user_agent())
            .build();
        ListenBrainz {
            base: base_url.trim_end_matches('/').to_owned(),
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Asks the server whether `token` is valid. `Ok(Some(user_name))` if it is.
    pub fn validate_token(&self, token: &str) -> Result<Option<String>, SubmitError> {
        let response = self
            .agent
            .get(format!("{}/1/validate-token", self.base))
            .header("Authorization", format!("Token {token}"))
            .call()
            .map_err(|_| SubmitError::Retry)?;
        let status = response.status().as_u16();
        if status == 401 {
            return Ok(None);
        }
        if status != 200 {
            return Err(classify(status));
        }
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|_| SubmitError::Retry)?;
        let value: Value = serde_json::from_str(&text).map_err(|_| SubmitError::Retry)?;
        if value["valid"].as_bool() == Some(true) {
            Ok(Some(
                value["user_name"].as_str().unwrap_or_default().to_owned(),
            ))
        } else {
            Ok(None)
        }
    }

    fn post(&self, token: &str, listen_type: &str, payload: Vec<Value>) -> Result<(), SubmitError> {
        let body = json!({"listen_type": listen_type, "payload": payload}).to_string();
        let response = self
            .agent
            .post(format!("{}/1/submit-listens", self.base))
            .header("Authorization", format!("Token {token}"))
            .content_type("application/json")
            .send(body)
            .map_err(|_| SubmitError::Retry)?;
        match response.status().as_u16() {
            200..=299 => Ok(()),
            401 | 403 => {
                let text = response.into_body().read_to_string().unwrap_or_default();
                Err(SubmitError::Auth(error_text(&text)))
            }
            status => Err(classify(status)),
        }
    }
}

/// The `error` field of a ListenBrainz error body, cut to a log-sized length.
fn error_text(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let text = value["error"].as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    const MAX: usize = 300;
    Some(match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{} [cut]", &text[..cut]),
        None => text.to_owned(),
    })
}

/// What a failed status means for the sender.
fn classify(status: u16) -> SubmitError {
    match status {
        401 | 403 => SubmitError::Auth(None),
        // 404 and 408 usually mean a wrong URL or a slow proxy: keep the data.
        404 | 408 | 429 | 500..=599 => SubmitError::Retry,
        _ => SubmitError::Rejected,
    }
}

/// The MBID in canonical lowercase form if it has the 8-4-4-4-12 hex shape. ListenBrainz
/// refuses a whole request over a malformed one, and tags in the wild are not clean.
fn normalize_mbid(raw: &str) -> Option<String> {
    let id = raw.trim().to_ascii_lowercase();
    let shaped = id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    shaped.then_some(id)
}

fn listen_json(listen: &Listen) -> Value {
    let mut info = Map::new();
    info.insert("submission_client".into(), json!("Armonika"));
    info.insert(
        "submission_client_version".into(),
        json!(armonika_core::VERSION),
    );
    if let Some(v) = listen.duration_ms.filter(|d| *d > 0) {
        info.insert("duration_ms".into(), json!(v));
    }
    if let Some(v) = listen.track_number {
        info.insert("tracknumber".into(), json!(v));
    }
    if let Some(v) = listen.recording_mbid.as_deref().and_then(normalize_mbid) {
        info.insert("recording_mbid".into(), json!(v));
    }
    let mut meta = Map::new();
    meta.insert("artist_name".into(), json!(listen.artist));
    meta.insert("track_name".into(), json!(listen.title));
    if let Some(v) = &listen.album {
        meta.insert("release_name".into(), json!(v));
    }
    meta.insert("additional_info".into(), Value::Object(info));
    let mut item = Map::new();
    if let Some(at) = listen.listened_at {
        item.insert("listened_at".into(), json!(at));
    }
    item.insert("track_metadata".into(), Value::Object(meta));
    Value::Object(item)
}

impl ScrobbleService for ListenBrainz {
    fn name(&self) -> &'static str {
        NAME
    }

    fn endpoint(&self) -> String {
        self.base.clone()
    }

    fn max_batch(&self) -> usize {
        MAX_BATCH
    }

    fn now_playing(&self, token: &str, listen: &Listen) -> Result<(), SubmitError> {
        let mut playing = listen.clone();
        playing.listened_at = None;
        self.post(token, "playing_now", vec![listen_json(&playing)])
    }

    fn submit(&self, token: &str, batch: &[Listen]) -> Result<(), SubmitError> {
        if batch.is_empty() {
            return Ok(());
        }
        let listen_type = if batch.len() == 1 { "single" } else { "import" };
        self.post(token, listen_type, batch.iter().map(listen_json).collect())
    }
}
