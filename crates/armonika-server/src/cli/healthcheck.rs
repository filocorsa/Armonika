//! `armonika healthcheck`: asks the local server whether it is alive, for the
//! container health check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, bail};

/// The address to call for a server bound to `bind`: an unspecified address
/// (`0.0.0.0`, `::`) means "every interface", so the loopback is used.
fn local_address(bind: SocketAddr) -> SocketAddr {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind.port())
}

/// Calls `GET /rest/ping` without credentials. The server is alive when it
/// answers `ok`, or with Subsonic error 10 (a required parameter is missing),
/// which is what it says to a ping that carries no login.
pub fn run(bind: SocketAddr) -> anyhow::Result<()> {
    let url = format!("http://{}/rest/ping?f=json", local_address(bind));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .call()
        .with_context(|| format!("no answer from {url}"))?;
    let status = response.status();
    if status != 200 {
        bail!("{url} answered HTTP {status}");
    }
    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .with_context(|| format!("{url} did not answer with JSON"))?;
    let reply = &body["subsonic-response"];
    if reply["status"] == "ok" || reply["error"]["code"] == 10 {
        return Ok(());
    }
    bail!("{url} answered an error: {reply}")
}
