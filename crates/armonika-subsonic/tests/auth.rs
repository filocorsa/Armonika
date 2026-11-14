mod common;

use std::time::{Duration, Instant};

use common::*;

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn token_auth_ok() {
    let app = app().await;
    let r = get(&app, &format!("/rest/ping?{}", login("ann", ANN_PASSWORD))).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "ok");
    // The user name is not case sensitive and the token may be uppercase.
    let t = token(ANN_PASSWORD, "salty").to_uppercase();
    let r = get(&app, &format!("/rest/ping?u=ANN&t={t}&s=salty&f=json")).await;
    assert_eq!(r.json()["status"], "ok");
}

#[tokio::test]
async fn plain_and_enc_password_ok() {
    let app = app().await;
    let r = get(&app, "/rest/ping?u=ann&p=ann-password&f=json").await;
    assert_eq!(r.json()["status"], "ok");
    let enc = hex(ANN_PASSWORD);
    let r = get(&app, &format!("/rest/ping?u=ann&p=enc:{enc}&f=json")).await;
    assert_eq!(r.json()["status"], "ok");
    let r = get(&app, "/rest/ping?u=ann&p=enc:zz&f=json").await;
    assert_eq!(r.json()["error"]["code"], 40);
}

#[tokio::test]
async fn api_key_ok() {
    let app = app().await;
    let r = get(&app, &format!("/rest/ping?apiKey={}&f=json", app.ann_key)).await;
    assert_eq!(r.json()["status"], "ok");
    let r = get(&app, "/rest/ping?apiKey=nope&f=json").await;
    let body = r.json();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["error"]["code"], 44);
}

#[tokio::test]
async fn wrong_password_is_40() {
    let app = app().await;
    let started = Instant::now();
    let r = get(
        &app,
        &format!("/rest/ping?{}", login("ann", "wrong-password")),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["error"]["code"], 40);
    assert!(started.elapsed() >= Duration::from_millis(500));
    // An unknown user gets the same answer.
    let r = get(
        &app,
        &format!("/rest/ping?{}", login("ghost", "whatever-pw")),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], 40);
}

#[tokio::test]
async fn failed_logins_do_not_serialize() {
    let app = app().await;
    let started = Instant::now();
    let uri = format!("/rest/ping?{}", login("ann", "wrong-password"));
    let (a, b, c) = tokio::join!(get(&app, &uri), get(&app, &uri), get(&app, &uri));
    assert_eq!(a.json()["error"]["code"], 40);
    assert_eq!(b.json()["error"]["code"], 40);
    assert_eq!(c.json()["error"]["code"], 40);
    // Three delays of 500 ms overlap; held one after the other they would take 1.5 s.
    assert!(started.elapsed() < Duration::from_millis(1200));
}

#[tokio::test]
async fn api_key_with_user_is_43() {
    let app = app().await;
    let r = get(
        &app,
        &format!(
            "/rest/ping?apiKey={}&u=ann&p=ann-password&f=json",
            app.ann_key
        ),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], 43);
}

#[tokio::test]
async fn missing_user_is_10() {
    let app = app().await;
    let r = get(&app, "/rest/ping?p=ann-password&f=json").await;
    assert_eq!(r.json()["error"]["code"], 10);
    let r = get(&app, "/rest/ping?u=ann&f=json").await;
    assert_eq!(r.json()["error"]["code"], 10);
    let r = get(&app, "/rest/ping?u=ann&t=abc&f=json").await;
    assert_eq!(r.json()["error"]["code"], 10);
}
