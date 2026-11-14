mod common;

use armonika_subsonic::Params;
use common::*;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn admin() -> String {
    login("admin", ADMIN_PASSWORD)
}

#[tokio::test]
async fn json_and_xml_envelopes() {
    let app = app().await;
    let j = get(&app, &format!("/rest/getMusicFolders?{}", ann())).await;
    let body = j.json();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], "1.16.1");
    assert_eq!(body["type"], "armonika");
    assert_eq!(body["serverVersion"], armonika_core::VERSION);
    assert_eq!(body["openSubsonic"], true);
    assert!(
        j.headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );

    let x = get(
        &app,
        &format!("/rest/getMusicFolders?{}", ann().replace("f=json", "f=xml")),
    )
    .await;
    let doc = roxmltree::Document::parse(&x.body).unwrap();
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "subsonic-response");
    assert_eq!(
        root.tag_name().namespace(),
        Some("http://subsonic.org/restapi")
    );
    assert_eq!(root.attribute("status"), Some("ok"));
    assert_eq!(root.attribute("version"), Some("1.16.1"));
    assert_eq!(root.attribute("type"), Some("armonika"));
    assert_eq!(root.attribute("openSubsonic"), Some("true"));
    let folders: Vec<_> = root
        .descendants()
        .filter(|n| n.tag_name().name() == "musicFolder")
        .collect();
    let json_folders = body["musicFolders"]["musicFolder"].as_array().unwrap();
    assert_eq!(folders.len(), json_folders.len());
    assert_eq!(folders[0].attribute("name"), Some("A"));
    assert_eq!(json_folders[0]["name"], "A");
    assert_eq!(
        folders[0].attribute("id").unwrap(),
        json_folders[0]["id"].to_string()
    );

    // XML is the default format; errors use the envelope too.
    let x = get(&app, "/rest/ping?u=ann&p=nope-nope").await;
    let doc = roxmltree::Document::parse(&x.body).unwrap();
    let error = doc
        .descendants()
        .find(|n| n.tag_name().name() == "error")
        .unwrap();
    assert_eq!(error.attribute("code"), Some("40"));
    assert_eq!(doc.root_element().attribute("status"), Some("failed"));
}

#[tokio::test]
async fn view_suffix_and_post_form_work() {
    let app = app().await;
    let r = get(&app, &format!("/rest/ping.view?{}", ann())).await;
    assert_eq!(r.json()["status"], "ok");
    // formPost: the credentials travel in the body.
    let r = post_form(&app, "/rest/ping.view", &ann()).await;
    assert_eq!(r.json()["status"], "ok");
    let r = post_form(&app, "/rest/getUser", &format!("{}&username=ann", ann())).await;
    assert_eq!(r.json()["user"]["username"], "ann");
    // Every response carries the server time.
    let timing = r.headers["server-timing"].to_str().unwrap();
    assert!(timing.starts_with("app;dur="), "{timing}");
}

#[test]
fn repeated_parameters_are_a_list() {
    let p = Params::parse(b"id=a&id=b&x=1&id=c%20d");
    assert_eq!(p.all("id"), ["a", "b", "c d"]);
    assert_eq!(p.get("x"), Some("1"));
    assert!(p.all("none").is_empty());
}

#[tokio::test]
async fn music_folders_only_visible() {
    let app = app().await;
    let r = get(&app, &format!("/rest/getMusicFolders?{}", ann())).await;
    let folders = r.json()["musicFolders"]["musicFolder"].clone();
    let folders = folders.as_array().unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0]["id"], app.lib_a.0);
    assert_eq!(folders[0]["name"], "A");

    let r = get(&app, &format!("/rest/getMusicFolders?{}", admin())).await;
    let folders = r.json()["musicFolders"]["musicFolder"].clone();
    assert_eq!(folders.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn get_user_reports_download_role() {
    let app = app().await;
    let r = get(&app, &format!("/rest/getUser?{}&username=ann", ann())).await;
    let user = r.json()["user"].clone();
    assert_eq!(user["username"], "ann");
    assert_eq!(user["downloadRole"], false);
    assert_eq!(user["adminRole"], false);
    assert_eq!(user["streamRole"], true);
    assert_eq!(user["scrobblingEnabled"], true);
    assert_eq!(user["folder"], serde_json::json!([app.lib_a.0]));

    // Another user: refused to a normal user, allowed to an admin.
    let r = get(&app, &format!("/rest/getUser?{}&username=admin", ann())).await;
    assert_eq!(r.json()["error"]["code"], 50);
    let r = get(&app, &format!("/rest/getUser?{}&username=ann", admin())).await;
    assert_eq!(r.json()["user"]["downloadRole"], false);
    let r = get(&app, &format!("/rest/getUser?{}&username=admin", admin())).await;
    let user = r.json()["user"].clone();
    assert_eq!(user["downloadRole"], true);
    assert_eq!(user["adminRole"], true);
    assert_eq!(user["folder"].as_array().unwrap().len(), 2);
    let r = get(&app, &format!("/rest/getUser?{}&username=ghost", admin())).await;
    assert_eq!(r.json()["error"]["code"], 70);
    let r = get(&app, &format!("/rest/getUser?{}", admin())).await;
    assert_eq!(r.json()["error"]["code"], 10);
}

#[tokio::test]
async fn start_scan_requires_manage() {
    let app = app().await;
    let r = get(&app, &format!("/rest/startScan?{}", ann())).await;
    assert_eq!(r.json()["error"]["code"], 50);

    let r = get(&app, &format!("/rest/startScan?{}", admin())).await;
    let body = r.json();
    assert_eq!(body["status"], "ok");
    assert!(body["scanStatus"]["scanning"].is_boolean());
    // The scan of the (empty) libraries ends; getScanStatus reports it.
    for _ in 0..200 {
        let r = get(&app, &format!("/rest/getScanStatus?{}", admin())).await;
        if r.json()["scanStatus"]["scanning"] == false {
            assert_eq!(r.json()["scanStatus"]["count"], 0);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the scan did not end");
}

#[tokio::test]
async fn license_and_extensions() {
    let app = app().await;
    let r = get(&app, &format!("/rest/getLicense?{}", ann())).await;
    assert_eq!(r.json()["license"]["valid"], true);
    let r = get(&app, &format!("/rest/getOpenSubsonicExtensions?{}", ann())).await;
    let ext = r.json()["openSubsonicExtensions"].clone();
    let names: Vec<_> = ext
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["formPost", "apiKeyAuthentication"]);
    assert_eq!(ext[0]["versions"], serde_json::json!([1]));
}

#[tokio::test]
async fn unknown_method_is_error_0() {
    let app = app().await;
    let r = get(&app, &format!("/rest/getPodcasts?{}", ann())).await;
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["error"]["code"], 0);
    assert_eq!(body["error"]["message"], "not implemented");
    // jsonp is not supported either.
    let r = get(
        &app,
        &format!(
            "/rest/ping?{}",
            ann().replace("f=json", "f=jsonp&callback=x")
        ),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], 0);
}
