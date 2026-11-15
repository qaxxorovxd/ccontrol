//! End-to-end tests: real TLS listener, real pairing state machine, fake audio adapters.

use ccontrol_agent::config::AgentConfig;
use ccontrol_agent::pairing::Decision;
use ccontrol_agent::state::{Adapters, AppState};
use ccontrol_protocol::*;
use ccontrol_security::{sas, tls, DeviceIdentity};
use ccontrol_system_control::{ControlError, MediaControl, TelemetrySampler, VolumeControl};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

struct FakeVolume(Mutex<VolumeState>);
impl VolumeControl for FakeVolume {
    fn get(&self) -> Result<VolumeState, ControlError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn set_level(&self, level: f32) -> Result<VolumeState, ControlError> {
        self.0.lock().unwrap().level = level;
        self.get()
    }
    fn set_muted(&self, muted: bool) -> Result<VolumeState, ControlError> {
        self.0.lock().unwrap().muted = muted;
        self.get()
    }
}
struct NoMedia;
impl MediaControl for NoMedia {
    fn send(&self, _: MediaKey) -> Result<(), ControlError> {
        Err(ControlError::Unsupported("test".into()))
    }
}

struct Agent {
    state: Arc<AppState>,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

async fn start_agent() -> Agent {
    let dir = tempfile::tempdir().unwrap();
    let identity = DeviceIdentity::generate("agent").unwrap();
    let store = ccontrol_persistence::Store::open(&dir.path().join("agent.db")).unwrap();
    let adapters = Adapters {
        volume: Box::new(FakeVolume(Mutex::new(VolumeState {
            level: 0.3,
            muted: false,
            endpoint_name: Some("Fake".into()),
        }))),
        media: Box::new(NoMedia),
        telemetry: TelemetrySampler::new(),
    };
    let cfg = AgentConfig {
        mdns_enabled: false,
        ..Default::default()
    };
    let state = AppState::new(
        dir.path().to_owned(),
        cfg,
        identity,
        store,
        adapters,
        "t".repeat(48),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ccontrol_agent::server::serve(state.clone(), listener));
    Agent {
        state,
        addr,
        _dir: dir,
    }
}

struct Client {
    rd: ReadHalf<TlsStream<TcpStream>>,
    wr: WriteHalf<TlsStream<TcpStream>>,
    next_id: u64,
}

impl Client {
    async fn connect(agent: &Agent, id: &DeviceIdentity) -> (Client, HelloAck) {
        Self::connect_with(
            agent,
            id,
            &agent.state.identity.fingerprint.clone(),
            PROTOCOL_VERSION,
        )
        .await
        .unwrap()
    }

    async fn connect_with(
        agent: &Agent,
        id: &DeviceIdentity,
        claimed_server: &str,
        version: u32,
    ) -> Result<(Client, HelloAck), ProtocolError> {
        let cfg = tls::client_config(id, Some(agent.state.identity.fingerprint.clone())).unwrap();
        let tcp = TcpStream::connect(agent.addr).await.unwrap();
        let stream = tokio_rustls::TlsConnector::from(cfg)
            .connect(
                rustls::pki_types::ServerName::try_from("ccontrol.invalid").unwrap(),
                tcp,
            )
            .await
            .unwrap();
        let (rd, wr) = tokio::io::split(stream);
        let mut c = Client { rd, wr, next_id: 0 };
        c.send(Message::Hello(Hello {
            protocol_version: version,
            min_protocol_version: 1,
            device_id: id.fingerprint.clone(),
            peer_device_id: claimed_server.into(),
            device_name: "Test Phone".into(),
            platform: "android".into(),
            app_version: "test".into(),
        }))
        .await;
        match c.recv().await {
            Message::HelloAck(a) => Ok((c, a)),
            Message::Error { error } => Err(error),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn send(&mut self, m: Message) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    async fn recv(&mut self) -> Message {
        tokio::time::timeout(Duration::from_secs(10), read_frame(&mut self.rd))
            .await
            .expect("timeout")
            .expect("frame")
    }

    async fn request(&mut self, action: Action) -> Result<ActionResult, ProtocolError> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(Message::Request { id, action }).await;
        loop {
            match self.recv().await {
                Message::Response {
                    id: rid,
                    result,
                    error,
                } if rid == id => {
                    return match (result, error) {
                        (Some(r), None) => Ok(r),
                        (_, Some(e)) => Err(e),
                        _ => panic!("empty response"),
                    }
                }
                Message::Event { .. } => continue,
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// Run the commit/reveal exchange; returns (client SAS, PairAwaitingApproval seen).
    async fn pair(&mut self, agent: &Agent, id: &DeviceIdentity, scopes: Vec<Scope>) -> String {
        let nc = sas::random_nonce();
        self.send(Message::PairCommit {
            commitment: sas::commitment(&nc),
            requested_scopes: scopes,
        })
        .await;
        let ns = match self.recv().await {
            Message::PairNonce { nonce } => hex::decode(nonce).unwrap(),
            other => panic!("unexpected {other:?}"),
        };
        self.send(Message::PairReveal {
            nonce: hex::encode(nc),
        })
        .await;
        assert!(matches!(
            self.recv().await,
            Message::PairAwaitingApproval { .. }
        ));
        sas::compute_sas(&agent.state.identity.fingerprint, &id.fingerprint, &nc, &ns)
    }
}

async fn wait_pending(agent: &Agent) -> ccontrol_agent::pairing::PendingView {
    for _ in 0..100 {
        if let Some(p) = agent.state.pairing.list().into_iter().next() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no pending request");
}

async fn paired_client(agent: &Agent, phone: &DeviceIdentity, scopes: Vec<Scope>) -> Client {
    let (mut c, ack) = Client::connect(agent, phone).await;
    assert!(!ack.paired);
    let my_sas = c.pair(agent, phone, scopes.clone()).await;
    let p = wait_pending(agent).await;
    assert_eq!(p.sas, my_sas, "both screens must show the same code");
    assert!(agent
        .state
        .pairing
        .decide(p.request_id, Decision::Accept(scopes)));
    match c.recv().await {
        Message::PairResult { accepted: true, .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    match c.recv().await {
        Message::HelloAck(a) => assert!(a.paired),
        other => panic!("unexpected {other:?}"),
    }
    c
}

#[tokio::test]
async fn pair_then_control_then_reconnect_without_pairing() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::DeviceRead, Scope::MediaControl]).await;

    match c.request(Action::DeviceInfo).await.unwrap() {
        ActionResult::Manifest { manifest } => {
            assert_eq!(manifest.device_id, agent.state.identity.fingerprint);
            assert!(manifest.capabilities["audio.master_volume"].available);
        }
        other => panic!("{other:?}"),
    }
    match c.request(Action::VolumeSet { level: 0.75 }).await.unwrap() {
        ActionResult::Volume(v) => assert_eq!(v.level, 0.75),
        other => panic!("{other:?}"),
    }
    // Not granted -> denied.
    let e = c.request(Action::TelemetryGet).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::ScopeDenied);
    // Invalid params.
    let e = c
        .request(Action::VolumeSet { level: 3.0 })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    drop(c);

    // Reconnect: trusted by key, no pairing needed, manifest delivered.
    let (mut c2, ack) = Client::connect(&agent, &phone).await;
    assert!(ack.paired);
    assert!(ack.manifest.is_some());
    match c2.request(Action::VolumeGet).await.unwrap() {
        ActionResult::Volume(v) => assert_eq!(v.level, 0.75),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn rejected_pairing_grants_nothing() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let (mut c, _) = Client::connect(&agent, &phone).await;
    c.pair(&agent, &phone, vec![Scope::MediaControl]).await;
    let p = wait_pending(&agent).await;
    agent
        .state
        .pairing
        .decide(p.request_id, Decision::Reject("no".into()));
    assert!(matches!(
        c.recv().await,
        Message::PairResult {
            accepted: false,
            ..
        }
    ));
    let (_, ack) = Client::connect(&agent, &phone).await;
    assert!(!ack.paired);
}

#[tokio::test]
async fn unpaired_client_cannot_send_requests() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let (mut c, ack) = Client::connect(&agent, &phone).await;
    assert!(!ack.paired);
    c.send(Message::Request {
        id: 1,
        action: Action::VolumeSet { level: 0.0 },
    })
    .await;
    match c.recv().await {
        Message::Error { error } => assert_eq!(error.code, ErrorCode::InvalidRequest),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(agent.state.adapters.volume.get().unwrap().level, 0.3);
}

#[tokio::test]
async fn revoked_controller_is_disconnected_and_rejected() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::MediaControl]).await;
    c.request(Action::VolumeGet).await.unwrap();

    agent.state.revoke(&phone.fingerprint, "test").unwrap();
    match c.recv().await {
        Message::Event {
            event: AgentEvent::Revoked { .. },
        } => {}
        other => panic!("unexpected {other:?}"),
    }
    let (_, ack) = Client::connect(&agent, &phone).await;
    assert!(!ack.paired, "revoked key must not be trusted anymore");
}

#[tokio::test]
async fn identity_claim_mismatch_is_rejected() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let err = Client::connect_with(&agent, &phone, &"0".repeat(64), PROTOCOL_VERSION)
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, ErrorCode::IdentityMismatch);
}

#[tokio::test]
async fn incompatible_protocol_is_reported() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let fp = agent.state.identity.fingerprint.clone();
    let err = Client::connect_with(&agent, &phone, &fp, 999)
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, ErrorCode::IncompatibleProtocol);
}

#[tokio::test]
async fn bad_commitment_fails_pairing() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let (mut c, _) = Client::connect(&agent, &phone).await;
    c.send(Message::PairCommit {
        commitment: sas::commitment(&[1; 32]),
        requested_scopes: vec![],
    })
    .await;
    assert!(matches!(c.recv().await, Message::PairNonce { .. }));
    c.send(Message::PairReveal {
        nonce: hex::encode([2u8; 32]),
    })
    .await;
    match c.recv().await {
        Message::Error { error } => assert_eq!(error.code, ErrorCode::PairingFailed),
        other => panic!("unexpected {other:?}"),
    }
    assert!(agent.state.pairing.list().is_empty());
}

#[tokio::test]
async fn duplicate_request_ids_are_rejected() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::MediaControl]).await;
    c.send(Message::Request {
        id: 42,
        action: Action::VolumeGet,
    })
    .await;
    assert!(matches!(
        c.recv().await,
        Message::Response {
            id: 42,
            error: None,
            ..
        }
    ));
    c.send(Message::Request {
        id: 42,
        action: Action::VolumeSet { level: 0.0 },
    })
    .await;
    match c.recv().await {
        Message::Response {
            id: 42,
            error: Some(e),
            ..
        } => assert_eq!(e.code, ErrorCode::InvalidRequest),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        agent.state.adapters.volume.get().unwrap().level,
        0.3,
        "replayed request must not execute"
    );
}

#[tokio::test]
async fn oversized_frame_closes_session() {
    use tokio::io::AsyncWriteExt;
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::MediaControl]).await;
    c.wr.write_all(&(MAX_FRAME_LEN as u32 + 1).to_be_bytes())
        .await
        .unwrap();
    c.wr.flush().await.unwrap();
    match c.recv().await {
        Message::Error { error } => assert_eq!(error.code, ErrorCode::InvalidRequest),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn ping_pong() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::DeviceRead]).await;
    c.send(Message::Ping { seq: 5 }).await;
    assert_eq!(c.recv().await, Message::Pong { seq: 5 });
}
