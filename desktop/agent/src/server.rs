//! Authenticated control listener and per-connection session state machine.

use crate::state::AppState;
use crate::{dispatch, net_policy, pairing};
use ccontrol_protocol::{
    is_compatible, read_frame, write_frame, AgentEvent, ErrorCode, FrameError, HelloAck, Message,
    ProtocolError, Scope, PROTOCOL_VERSION,
};
use ccontrol_security::{short_fingerprint, tls::peer_fingerprint};
use std::collections::{HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio_rustls::TlsAcceptor;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Controllers ping every 30 s while in the foreground; drop silent links.
const IDLE_TIMEOUT: Duration = Duration::from_secs(95);
const MAX_CONNECTIONS: usize = 64;
const MAX_INFLIGHT_PER_SESSION: usize = 8;
const DEDUP_WINDOW: usize = 512;

pub async fn serve(state: Arc<AppState>, listener: TcpListener) -> anyhow::Result<()> {
    let acceptor = TlsAcceptor::from(ccontrol_security::tls::server_config(&state.identity)?);
    let conn_limit = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if !state.config.allow_public_peers && !net_policy::is_private_peer(peer.ip()) {
            tracing::warn!(%peer, "rejected connection from non-private address");
            continue;
        }
        let Ok(permit) = conn_limit.clone().try_acquire_owned() else {
            tracing::warn!(%peer, "connection limit reached");
            continue;
        };
        let _ = tcp.set_nodelay(true);
        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = handle_connection(state, acceptor, tcp, peer).await {
                tracing::debug!(%peer, error = %e, "connection ended with error");
            }
        });
    }
}

async fn send_fatal<W: tokio::io::AsyncWrite + Unpin>(wr: &mut W, code: ErrorCode, msg: &str) {
    let _ = write_frame(
        wr,
        &Message::Error {
            error: ProtocolError::new(code, msg),
        },
    )
    .await;
}

pub async fn handle_connection(
    state: Arc<AppState>,
    acceptor: TlsAcceptor,
    tcp: TcpStream,
    peer: SocketAddr,
) -> anyhow::Result<()> {
    let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await??;
    let client_fp = peer_fingerprint(tls.get_ref().1.peer_certificates())?;
    let (mut rd, mut wr) = tokio::io::split(tls);

    let hello = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut rd)).await {
        Ok(Ok(Message::Hello(h))) => h,
        Ok(Ok(_)) => {
            send_fatal(&mut wr, ErrorCode::InvalidRequest, "expected hello").await;
            return Ok(());
        }
        _ => return Ok(()),
    };
    if !is_compatible(hello.protocol_version) {
        send_fatal(
            &mut wr,
            ErrorCode::IncompatibleProtocol,
            &format!(
                "agent speaks protocol {PROTOCOL_VERSION}, controller {}",
                hello.protocol_version
            ),
        )
        .await;
        return Ok(());
    }
    // Both sides must agree on who is talking to whom. A mismatch means the
    // controller is behind an intercepting proxy or computes fingerprints differently.
    if !hello.device_id.eq_ignore_ascii_case(&client_fp)
        || !hello
            .peer_device_id
            .eq_ignore_ascii_case(&state.identity.fingerprint)
    {
        let _ = state
            .store
            .audit(&client_fp, "session.hello", "identity_mismatch");
        send_fatal(
            &mut wr,
            ErrorCode::IdentityMismatch,
            "certificate fingerprints do not match hello",
        )
        .await;
        return Ok(());
    }

    let controller = match state.store.get_controller(&client_fp)? {
        Some(c) => c,
        None => {
            write_frame(
                &mut wr,
                &Message::HelloAck(HelloAck {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: state.identity.fingerprint.clone(),
                    device_name: state.device_name.clone(),
                    paired: false,
                    granted_scopes: vec![],
                    manifest: None,
                }),
            )
            .await?;
            match pairing::run(&state, &mut rd, &mut wr, &hello, &client_fp, peer.ip()).await? {
                Some(c) => c,
                None => return Ok(()),
            }
        }
    };

    let _ = state.store.touch(&client_fp);
    let manifest = controller
        .scopes
        .contains(&Scope::DeviceRead)
        .then(|| state.manifest());
    write_frame(
        &mut wr,
        &Message::HelloAck(HelloAck {
            protocol_version: PROTOCOL_VERSION,
            device_id: state.identity.fingerprint.clone(),
            device_name: state.device_name.clone(),
            paired: true,
            granted_scopes: controller.scopes.clone(),
            manifest,
        }),
    )
    .await?;
    tracing::info!(%peer, device = %short_fingerprint(&client_fp), name = %controller.name, "controller session started");
    let _ = state.store.audit(&client_fp, "session.start", "ok");

    run_session(state.clone(), rd, wr, client_fp.clone(), peer).await;
    let _ = state.store.touch(&client_fp);
    tracing::info!(%peer, device = %short_fingerprint(&client_fp), "controller session ended");
    Ok(())
}

async fn run_session<R, W>(
    state: Arc<AppState>,
    mut rd: R,
    mut wr: W,
    client_fp: String,
    peer: SocketAddr,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (_guard, mut control_rx) = state.sessions.register(&client_fp, peer);

    // Single writer task fed by a bounded queue (backpressure).
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(64);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if write_frame(&mut wr, &m).await.is_err() {
                break;
            }
        }
    });

    let inflight = Arc::new(Semaphore::new(MAX_INFLIGHT_PER_SESSION));
    let mut seen: (HashSet<u64>, VecDeque<u64>) = (HashSet::new(), VecDeque::new());

    loop {
        tokio::select! {
            ctl = control_rx.recv() => {
                if let Some(crate::sessions::SessionControl::Kick { reason }) = ctl {
                    let _ = out_tx.send(Message::Event { event: AgentEvent::Revoked { reason } }).await;
                }
                break;
            }
            frame = tokio::time::timeout(IDLE_TIMEOUT, read_frame(&mut rd)) => {
                let msg = match frame {
                    Err(_) => break, // idle
                    Ok(Err(FrameError::TooLarge(_))) | Ok(Err(FrameError::Malformed(_))) | Ok(Err(FrameError::Empty)) => {
                        let _ = out_tx.send(Message::Error { error: ProtocolError::new(ErrorCode::InvalidRequest, "malformed or oversized frame") }).await;
                        break;
                    }
                    Ok(Err(_)) => break,
                    Ok(Ok(m)) => m,
                };
                match msg {
                    Message::Ping { seq } => { let _ = out_tx.send(Message::Pong { seq }).await; }
                    Message::Request { id, action } => {
                        // Replay / duplicate protection inside the session.
                        if !seen.0.insert(id) {
                            let _ = out_tx.send(Message::Response { id, result: None, error: Some(ProtocolError::new(ErrorCode::InvalidRequest, "duplicate request id")) }).await;
                            continue;
                        }
                        seen.1.push_back(id);
                        if seen.1.len() > DEDUP_WINDOW {
                            if let Some(old) = seen.1.pop_front() { seen.0.remove(&old); }
                        }
                        // Re-read trust on every request so revocation and scope
                        // changes apply immediately.
                        let scopes = match state.store.get_controller(&client_fp) {
                            Ok(Some(c)) => c.scopes,
                            _ => {
                                let _ = out_tx.send(Message::Event { event: AgentEvent::Revoked { reason: "pairing no longer trusted".into() } }).await;
                                break;
                            }
                        };
                        let Ok(permit) = inflight.clone().try_acquire_owned() else {
                            let _ = out_tx.send(Message::Response { id, result: None, error: Some(ProtocolError::new(ErrorCode::Busy, "too many requests in flight")) }).await;
                            continue;
                        };
                        let state = state.clone();
                        let out = out_tx.clone();
                        let fp = client_fp.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let name = action.name();
                            let r = dispatch::execute(&state, &fp, &scopes, action).await;
                            let outcome = match &r { Ok(_) => "ok".to_owned(), Err(e) => format!("{:?}", e.code) };
                            if !matches!(name, "telemetry.get" | "volume.get" | "device.info" | "media.status" | "process.list" | "app.list" | "route.list" | "route.lease.renew" | "audio.devices") {
                                let _ = state.store.audit(&fp, name, &outcome);
                            }
                            let msg = match r {
                                Ok(res) => Message::Response { id, result: Some(res), error: None },
                                Err(e) => Message::Response { id, result: None, error: Some(e) },
                            };
                            let _ = out.send(msg).await;
                        });
                    }
                    _ => {
                        let _ = out_tx.send(Message::Error { error: ProtocolError::new(ErrorCode::InvalidRequest, "unexpected message in session") }).await;
                        break;
                    }
                }
            }
        }
    }
    drop(out_tx);
    // Give the writer a moment to flush the final event.
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}
