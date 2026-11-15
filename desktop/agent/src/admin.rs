//! Loopback-only settings UI / local admin API.
//!
//! Bound to 127.0.0.1 only. Every API call requires the per-install admin
//! token (kept in the user's data directory) and a loopback `Host` header,
//! which blocks DNS-rebinding and cross-site requests from web pages.

use crate::pairing::Decision;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ccontrol_protocol::Scope;
use ccontrol_security::short_fingerprint;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const PAGE: &str = include_str!("admin.html");

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/state", get(api_state))
        .route("/api/pairing/{id}/accept", post(api_accept))
        .route("/api/pairing/{id}/reject", post(api_reject))
        .route("/api/controllers/{id}/revoke", post(api_revoke))
        .route("/api/controllers/{id}/scopes", post(api_scopes))
        .route("/api/routes/{id}/stop", post(api_route_stop))
        .with_state(state)
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let host_ok = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let port = state.config.admin_port;
            h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
        })
        .unwrap_or(false);
    let token_ok = headers
        .get("x-ccontrol-token")
        .and_then(|h| h.to_str().ok())
        .map(|t| constant_eq(t.as_bytes(), state.admin_token.as_bytes()))
        .unwrap_or(false);
    host_ok && token_ok
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn deny() -> Response {
    (StatusCode::FORBIDDEN, "forbidden").into_response()
}

#[derive(Serialize)]
struct ControllerView {
    device_id: String,
    short_id: String,
    name: String,
    platform: String,
    scopes: Vec<Scope>,
    paired_at: i64,
    last_seen: Option<i64>,
    online: bool,
}

#[derive(Serialize)]
struct StateView {
    device_name: String,
    device_id: String,
    short_id: String,
    port: u16,
    addresses: Vec<String>,
    pairing_uri: Option<String>,
    pairing_qr_svg: Option<String>,
    manifest: ccontrol_protocol::CapabilityManifest,
    pending: Vec<crate::pairing::PendingView>,
    controllers: Vec<ControllerView>,
    sessions: Vec<crate::sessions::SessionInfo>,
    audit: Vec<AuditView>,
    all_scopes: Vec<ScopeView>,
    routes: Vec<ccontrol_protocol::RouteInfo>,
    sinks: Vec<ccontrol_protocol::SinkInfo>,
}

#[derive(Serialize)]
struct ScopeView {
    id: &'static str,
    high_risk: bool,
}

#[derive(Serialize)]
struct AuditView {
    ts: i64,
    short_id: String,
    action: String,
    outcome: String,
}

/// Non-loopback IPv4 addresses that controllers can reach.
pub fn lan_addresses() -> Vec<std::net::IpAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| ip.is_ipv4() && crate::net_policy::is_private_peer(*ip))
        .collect()
}

pub fn pairing_uri(state: &AppState, host: &std::net::IpAddr) -> String {
    let name: String = state
        .device_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "ccontrol://pair?h={host}&p={}&fp={}&n={name}",
        state.config.listen_port, state.identity.fingerprint
    )
}

async fn api_state(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    let addrs = lan_addresses();
    let uri = addrs.first().map(|a| pairing_uri(&state, a));
    let qr = uri.as_ref().and_then(|u| {
        qrcode::QrCode::new(u.as_bytes()).ok().map(|q| {
            q.render::<qrcode::render::svg::Color>()
                .min_dimensions(220, 220)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build()
        })
    });
    let controllers = state
        .store
        .list_controllers()
        .unwrap_or_default()
        .into_iter()
        .map(|c| ControllerView {
            short_id: short_fingerprint(&c.device_id),
            online: state.sessions.is_online(&c.device_id),
            device_id: c.device_id,
            name: c.name,
            platform: c.platform,
            scopes: c.scopes,
            paired_at: c.paired_at,
            last_seen: c.last_seen,
        })
        .collect();
    let audit = state
        .store
        .recent_audit(30)
        .unwrap_or_default()
        .into_iter()
        .map(|a| AuditView {
            ts: a.ts,
            short_id: short_fingerprint(&a.device_id),
            action: a.action,
            outcome: a.outcome,
        })
        .collect();
    Json(StateView {
        device_name: state.device_name.clone(),
        device_id: state.identity.fingerprint.clone(),
        short_id: short_fingerprint(&state.identity.fingerprint),
        port: state.config.listen_port,
        addresses: addrs.iter().map(|a| a.to_string()).collect(),
        pairing_uri: uri,
        pairing_qr_svg: qr,
        manifest: state.manifest(),
        pending: state.pairing.list(),
        controllers,
        sessions: state.sessions.list(),
        audit,
        all_scopes: Scope::ALL
            .iter()
            .map(|s| ScopeView {
                id: s.as_str(),
                high_risk: s.is_high_risk(),
            })
            .collect(),
        routes: state.audio.list().0,
        sinks: state.audio.list().1,
    })
    .into_response()
}

#[derive(Deserialize)]
struct ScopesBody {
    scopes: Vec<String>,
}

fn parse_scopes(v: &[String]) -> Vec<Scope> {
    v.iter().filter_map(|s| Scope::parse(s)).collect()
}

async fn api_accept(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(body): Json<ScopesBody>,
) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    if state
        .pairing
        .decide(id, Decision::Accept(parse_scopes(&body.scopes)))
    {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, "request expired").into_response()
    }
}

async fn api_reject(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    state
        .pairing
        .decide(id, Decision::Reject("rejected by the desktop owner".into()));
    StatusCode::NO_CONTENT.into_response()
}

async fn api_revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    match state.revoke(&id, "revoked by the desktop owner") {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Local, manual stop/restore of a route or sink (owner override on this computer).
async fn api_route_stop(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    let r = state
        .audio
        .stop_route(&state, &id, "the owner on this computer")
        .await;
    let s = state
        .audio
        .stop_sink(&state, &id, Some("stopped on this computer".into()))
        .await;
    if r.is_err() && s.is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let _ = state.store.audit("local", "route.stop", &id);
    StatusCode::NO_CONTENT.into_response()
}

async fn api_scopes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ScopesBody>,
) -> Response {
    if !authorized(&state, &headers) {
        return deny();
    }
    match state.store.set_scopes(&id, &parse_scopes(&body.scopes)) {
        Ok(true) => {
            let _ = state.store.audit(&id, "pairing.scopes_changed", "ok");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub fn admin_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/#token={token}")
}

/// Load the admin token, creating one on first run.
pub fn load_or_create_token(dir: &std::path::Path) -> anyhow::Result<String> {
    let p = dir.join("admin.token");
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_owned();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let t = hex::encode(b);
    ccontrol_security::protect::write_private(&p, t.as_bytes())?;
    Ok(t)
}
