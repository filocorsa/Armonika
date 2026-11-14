//! Authentication of a request: token (`u`, `t`, `s`), password (`u`, `p`, plain or
//! `enc:` hex) or API key (`apiKey`, extension `apiKeyAuthentication`).
//!
//! Token and password are checked against the Subsonic password of the user (the one
//! the token scheme needs in clear), never against the login password.

use armonika_core::access::Access;
use armonika_core::users;
use md5::{Digest, Md5};
use subtle::ConstantTimeEq;

use crate::error::ApiError;
use crate::{AppState, Params};

/// Who makes the request, with what they may reach.
#[derive(Clone, Debug)]
pub struct Caller(pub Access);

/// Authenticates the request described by `params`.
pub async fn authenticate(state: &AppState, params: &Params) -> Result<Caller, ApiError> {
    if let Some(api_key) = params.get("apiKey") {
        if ["u", "p", "t"].iter().any(|k| params.get(k).is_some()) {
            return Err(ApiError::conflicting_authentication());
        }
        let api_key = api_key.to_owned();
        let access = state
            .db
            .read(move |c| match users::user_by_api_key(c, &api_key)? {
                Some(id) => Access::load(c, &id).map(Some),
                None => Ok(None),
            })
            .await?;
        return access.map(Caller).ok_or_else(ApiError::invalid_api_key);
    }

    let name = params.require("u")?.to_owned();
    let proof = if let Some(token) = params.get("t") {
        Proof::Token {
            token: token.to_ascii_lowercase(),
            salt: params.require("s")?.to_owned(),
        }
    } else if let Some(password) = params.get("p") {
        Proof::Password(decode_password(password).ok_or_else(ApiError::wrong_credentials)?)
    } else {
        return Err(ApiError::missing_parameter("p"));
    };

    let secrets = state.secrets.clone();
    let found = state
        .db
        .read(move |c| {
            let Some(user) = users::by_name(c, &name)? else {
                return Ok(None);
            };
            let password = users::subsonic_password(c, &secrets, &user.id)?;
            Ok(Some((password, Access::load(c, &user.id)?)))
        })
        .await?;
    // An unknown user goes through the same comparison, against a password nobody has.
    let (password, access) = match found {
        Some((password, access)) => (password, Some(access)),
        None => ("no such user".to_owned(), None),
    };
    let matches = proof.matches(&password);
    match access {
        Some(access) if matches => Ok(Caller(access)),
        _ => Err(ApiError::wrong_credentials()),
    }
}

enum Proof {
    Token { token: String, salt: String },
    Password(String),
}

impl Proof {
    /// Whether this proof is valid for the Subsonic password `password`. The
    /// comparison takes the same time wherever the values differ.
    fn matches(&self, password: &str) -> bool {
        match self {
            Proof::Token { token, salt } => {
                let expected = hex(&Md5::digest(format!("{password}{salt}")));
                expected.as_bytes().ct_eq(token.as_bytes()).into()
            }
            Proof::Password(given) => {
                // Digests have a fixed length, so the lengths of the passwords do not
                // show in the time either.
                let a = Md5::digest(given.as_bytes());
                let b = Md5::digest(password.as_bytes());
                a.as_slice().ct_eq(b.as_slice()).into()
            }
        }
    }
}

/// A plain password, or the text of a password sent as `enc:` and hex.
fn decode_password(p: &str) -> Option<String> {
    let Some(encoded) = p.strip_prefix("enc:") else {
        return Some(p.to_owned());
    };
    if encoded.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(encoded.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
