//! Pairing: commit/reveal SAS exchange + explicit owner approval on the desktop.

use crate::state::AppState;
use ccontrol_persistence::TrustedController;
use ccontrol_protocol::{read_frame, write_frame, ErrorCode, Hello, Message, ProtocolError, Scope};
use ccontrol_security::sas;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;

pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PENDING: usize = 4;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(600);
const MAX_ATTEMPTS_PER_IP: usize = 6;

#[derive(Debug)]
pub enum Decision {
    Accept(Vec<Scope>),
    Reject(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingView {
    pub request_id: u64,
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub peer_ip: String,
    pub sas: String,
    pub requested_scopes: Vec<Scope>,
    pub expires_in_secs: u64,
}

struct Pending {
    view: PendingView,
    created: Instant,
    tx: oneshot::Sender<Decision>,
}

#[derive(Default)]
pub struct PairingManager {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    attempts: HashMap<IpAddr, VecDeque<Instant>>,
}

impl PairingManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Per-source-IP throttling of pairing attempts.
    pub fn allow_attempt(&self, ip: IpAddr) -> bool {
        let mut g = self.lock();
        let q = g.attempts.entry(ip).or_default();
        while q.front().is_some_and(|t| t.elapsed() > ATTEMPT_WINDOW) {
            q.pop_front();
        }
        if q.len() >= MAX_ATTEMPTS_PER_IP {
            return false;
        }
        q.push_back(Instant::now());
        true
    }

    fn add(&self, mut view: PendingView) -> Option<(u64, oneshot::Receiver<Decision>)> {
        let mut g = self.lock();
        if g.pending.len() >= MAX_PENDING {
            return None;
        }
        g.next_id += 1;
        let id = g.next_id;
        view.request_id = id;
        let (tx, rx) = oneshot::channel();
        g.pending.insert(
            id,
            Pending {
                view,
                created: Instant::now(),
                tx,
            },
        );
        Some((id, rx))
    }

    fn remove(&self, id: u64) {
        self.lock().pending.remove(&id);
    }

    /// Owner decision from the local settings UI.
    pub fn decide(&self, request_id: u64, decision: Decision) -> bool {
        match self.lock().pending.remove(&request_id) {
            Some(p) => p.tx.send(decision).is_ok(),
            None => false,
        }
    }

    pub fn list(&self) -> Vec<PendingView> {
        self.lock()
            .pending
            .values()
            .map(|p| {
                let mut v = p.view.clone();
                v.expires_in_secs = APPROVAL_TIMEOUT
                    .saturating_sub(p.created.elapsed())
                    .as_secs();
                v
            })
            .collect()
    }
}

fn sanitize_name(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(64).collect();
    if t.trim().is_empty() {
        "Unnamed device".into()
    } else {
        t.trim().to_owned()
    }
}

fn decode_nonce(hex_str: &str) -> Option<Vec<u8>> {
    let v = hex::decode(hex_str).ok()?;
    (v.len() == sas::NONCE_LEN).then_some(v)
}

async fn fail<W: AsyncWrite + Unpin>(
    wr: &mut W,
    code: ErrorCode,
    msg: &str,
) -> anyhow::Result<Option<TrustedController>> {
    let _ = write_frame(
        wr,
        &Message::Error {
            error: ProtocolError::new(code, msg),
        },
    )
    .await;
    Ok(None)
}

/// Run the pairing exchange. Returns the new trusted controller on approval.
pub async fn run<R, W>(
    state: &AppState,
    rd: &mut R,
    wr: &mut W,
    hello: &Hello,
    client_fp: &str,
    peer_ip: IpAddr,
) -> anyhow::Result<Option<TrustedController>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    use tokio::time::timeout;

    if !state.pairing.allow_attempt(peer_ip) {
        return fail(
            wr,
            ErrorCode::RateLimited,
            "too many pairing attempts; try again later",
        )
        .await;
    }

    let (commitment, mut requested) = match timeout(STEP_TIMEOUT, read_frame(rd)).await {
        Ok(Ok(Message::PairCommit {
            commitment,
            requested_scopes,
        })) => (commitment, requested_scopes),
        Ok(Ok(_)) => return fail(wr, ErrorCode::InvalidRequest, "expected pair_commit").await,
        _ => return Ok(None),
    };
    if commitment.len() != 64 || hex::decode(&commitment).is_err() {
        return fail(wr, ErrorCode::InvalidRequest, "malformed commitment").await;
    }
    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        requested = Scope::LOW_RISK_DEFAULT.to_vec();
    }

    let server_nonce = sas::random_nonce();
    write_frame(
        wr,
        &Message::PairNonce {
            nonce: hex::encode(server_nonce),
        },
    )
    .await?;

    let client_nonce = match timeout(STEP_TIMEOUT, read_frame(rd)).await {
        Ok(Ok(Message::PairReveal { nonce })) => match decode_nonce(&nonce) {
            Some(n) => n,
            None => return fail(wr, ErrorCode::InvalidRequest, "malformed nonce").await,
        },
        Ok(Ok(_)) => return fail(wr, ErrorCode::InvalidRequest, "expected pair_reveal").await,
        _ => return Ok(None),
    };
    if !sas::verify_commitment(&commitment, &client_nonce) {
        let _ = state
            .store
            .audit(client_fp, "pairing.request", "commitment_mismatch");
        return fail(
            wr,
            ErrorCode::PairingFailed,
            "nonce does not match commitment",
        )
        .await;
    }

    let code = sas::compute_sas(
        &state.identity.fingerprint,
        client_fp,
        &client_nonce,
        &server_nonce,
    );
    let view = PendingView {
        request_id: 0,
        device_id: client_fp.to_owned(),
        device_name: sanitize_name(&hello.device_name),
        platform: sanitize_name(&hello.platform),
        peer_ip: peer_ip.to_string(),
        sas: code,
        requested_scopes: requested.clone(),
        expires_in_secs: APPROVAL_TIMEOUT.as_secs(),
    };
    let Some((req_id, rx)) = state.pairing.add(view.clone()) else {
        return fail(wr, ErrorCode::Busy, "too many pending pairing requests").await;
    };
    tracing::info!(device = %ccontrol_security::short_fingerprint(client_fp), name = %view.device_name, "pairing request awaiting owner approval");
    let _ = state.store.audit(client_fp, "pairing.request", "pending");

    write_frame(
        wr,
        &Message::PairAwaitingApproval {
            timeout_secs: APPROVAL_TIMEOUT.as_secs() as u32,
        },
    )
    .await?;

    let decision = tokio::select! {
        d = rx => d.unwrap_or(Decision::Reject("cancelled".into())),
        _ = tokio::time::sleep(APPROVAL_TIMEOUT) => {
            state.pairing.remove(req_id);
            let _ = state.store.audit(client_fp, "pairing.request", "timeout");
            return fail(wr, ErrorCode::PairingTimeout, "the desktop owner did not respond in time").await;
        }
        // Controller cancelled or disconnected while waiting.
        _ = read_frame(rd) => {
            state.pairing.remove(req_id);
            let _ = state.store.audit(client_fp, "pairing.request", "cancelled_by_controller");
            return Ok(None);
        }
    };

    match decision {
        Decision::Accept(granted) => {
            let granted: Vec<Scope> = granted
                .into_iter()
                .filter(|s| requested.contains(s))
                .collect();
            let c = TrustedController {
                device_id: client_fp.to_owned(),
                name: view.device_name,
                platform: view.platform,
                scopes: granted.clone(),
                paired_at: ccontrol_persistence::now(),
                last_seen: Some(ccontrol_persistence::now()),
            };
            state.store.upsert_controller(&c)?;
            let _ = state.store.audit(client_fp, "pairing.request", "accepted");
            write_frame(
                wr,
                &Message::PairResult {
                    accepted: true,
                    granted_scopes: granted,
                    reason: None,
                },
            )
            .await?;
            Ok(Some(c))
        }
        Decision::Reject(reason) => {
            let _ = state.store.audit(client_fp, "pairing.request", "rejected");
            write_frame(
                wr,
                &Message::PairResult {
                    accepted: false,
                    granted_scopes: vec![],
                    reason: Some(reason),
                },
            )
            .await?;
            Ok(None)
        }
    }
}
