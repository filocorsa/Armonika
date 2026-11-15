//! System endpoints: `ping`, `getLicense`, `getOpenSubsonicExtensions`,
//! `getMusicFolders`, `getUser`, `getScanStatus` and `startScan`.

use armonika_core::access::{Access, Permission};
use armonika_core::ids::LibraryId;
use armonika_core::{libraries, users};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::response::Payload;
use crate::{AppState, Params};

pub fn ping() -> Payload {
    Payload::empty()
}

pub fn get_license() -> Payload {
    Payload("license", json!({ "valid": true }))
}

pub fn get_open_subsonic_extensions() -> Payload {
    Payload(
        "openSubsonicExtensions",
        json!([
            { "name": "formPost", "versions": [1] },
            { "name": "apiKeyAuthentication", "versions": [1] },
        ]),
    )
}

pub async fn get_music_folders(state: &AppState, access: &Access) -> Result<Payload, ApiError> {
    let visible = access.visible(None);
    let all = state.db.read(libraries::list).await?;
    let folders: Vec<Value> = all
        .into_iter()
        .filter(|l| visible.contains(&l.id))
        .map(|l| json!({ "id": l.id.0, "name": l.name }))
        .collect();
    Ok(Payload("musicFolders", json!({ "musicFolder": folders })))
}

pub async fn get_user(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let username = params.require("username")?;
    let target = if username.eq_ignore_ascii_case(&access.name) {
        access.clone()
    } else if access.is_admin {
        let name = username.to_owned();
        let found = state
            .db
            .read(move |c| match users::by_name(c, &name)? {
                Some(u) => Access::load(c, &u.id).map(Some),
                None => Ok(None),
            })
            .await?;
        found.ok_or_else(ApiError::not_found)?
    } else {
        return Err(ApiError::unauthorized());
    };
    let folders: Vec<i64> = target.visible(None).iter().map(|l| l.0).collect();
    Ok(Payload(
        "user",
        json!({
            "username": target.name,
            "scrobblingEnabled": true,
            "adminRole": target.is_admin,
            "settingsRole": false,
            "downloadRole": target.can_download,
            "uploadRole": false,
            "playlistRole": true,
            "coverArtRole": false,
            "commentRole": false,
            "podcastRole": false,
            "streamRole": true,
            "jukeboxRole": false,
            "shareRole": false,
            "videoConversionRole": false,
            "folder": folders,
        }),
    ))
}

pub fn get_scan_status(state: &AppState, access: &Access) -> Payload {
    scan_status(state, &access.visible(None))
}

/// Starts the scan of the libraries the caller manages.
pub fn start_scan(state: &AppState, access: &Access) -> Result<Payload, ApiError> {
    let managed: Vec<LibraryId> = access
        .libraries
        .iter()
        .filter(|&&(_, p)| p == Permission::Manage)
        .map(|&(id, _)| id)
        .collect();
    if managed.is_empty() {
        return Err(ApiError::unauthorized());
    }
    for id in &managed {
        state.scans.request(*id);
    }
    Ok(scan_status(state, &managed))
}

/// The state of the scans of `libraries`: whether one runs and the files seen.
fn scan_status(state: &AppState, libraries: &[LibraryId]) -> Payload {
    let status = state.scans.status();
    let ours = status
        .libraries
        .iter()
        .filter(|(id, _)| libraries.contains(id));
    let scanning = ours.clone().any(|(_, s)| s.running);
    let count: u64 = ours.map(|(_, s)| s.files_seen).sum();
    Payload(
        "scanStatus",
        json!({ "scanning": scanning, "count": count }),
    )
}
