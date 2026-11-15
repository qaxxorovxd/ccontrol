//! End-to-end tests: real TLS listener, real pairing state machine, fake audio adapters.

use ccontrol_agent::config::AgentConfig;
use ccontrol_agent::pairing::Decision;
use ccontrol_agent::state::{Adapters, AppState};
use ccontrol_protocol::*;
use ccontrol_security::{sas, tls, DeviceIdentity};
use ccontrol_system_control::{
    ControlError, MediaControl, ProcessManager, TelemetrySampler, VolumeControl, WindowControl,
};
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
    fn status(&self) -> Result<MediaStatus, ControlError> {
        Ok(MediaStatus {
            active: true,
            app: Some("Spotify".into()),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            status: PlaybackStatus::Playing,
            position_ms: Some(1000),
            duration_ms: Some(180_000),
            can_play_pause: true,
            can_next: true,
            can_previous: false,
        })
    }
}

#[derive(Default)]
struct FakeWindows(Mutex<Vec<u64>>);
impl WindowControl for FakeWindows {
    fn list(&self) -> Result<Vec<AppWindow>, ControlError> {
        Ok(vec![AppWindow {
            window_id: 77,
            pid: 4242,
            process_name: "notepad.exe".into(),
            title: "notes.txt - Notepad".into(),
            focused: true,
            minimized: false,
        }])
    }
    fn close(&self, window_id: u64, pid: u32) -> Result<(), ControlError> {
        if window_id != 77 || pid != 4242 {
            return Err(ControlError::Invalid("stale".into()));
        }
        self.0.lock().unwrap().push(window_id);
        Ok(())
    }
    fn close_all_of(&self, _: u32) -> Result<usize, ControlError> {
        Ok(0)
    }
}

struct Agent {
    audio: Arc<FakeAudio>,
    state: Arc<AppState>,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

async fn start_agent() -> Agent {
    start_agent_with(Arc::new(FakeAudio::new())).await
}

async fn start_agent_with(audio: Arc<FakeAudio>) -> Agent {
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
        processes: ProcessManager::new(),
        windows: Box::new(FakeWindows::default()),
        telemetry: TelemetrySampler::new(),
        audio: audio.clone(),
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
        audio,
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

#[tokio::test]
async fn media_status_reports_now_playing() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::MediaControl]).await;
    match c.request(Action::MediaStatus).await.unwrap() {
        ActionResult::MediaStatus(m) => {
            assert!(m.active);
            assert_eq!(m.title.as_deref(), Some("Song"));
            assert_eq!(m.status, PlaybackStatus::Playing);
        }
        other => panic!("{other:?}"),
    }
}

fn spawn_sleeper() -> std::process::Child {
    #[cfg(windows)]
    return std::process::Command::new("ping")
        .args(["-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    #[cfg(not(windows))]
    return std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
}

#[tokio::test]
async fn process_list_and_terminate_with_safeguards() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut child = spawn_sleeper();

    // device.read only: can list, cannot terminate.
    let mut c = paired_client(&agent, &phone, vec![Scope::DeviceRead]).await;
    let list = match c.request(Action::ProcessList).await.unwrap() {
        ActionResult::Processes { processes } => processes,
        other => panic!("{other:?}"),
    };
    let me = list
        .iter()
        .find(|p| p.pid == std::process::id())
        .expect("agent listed");
    assert!(!me.terminable);
    let target = list
        .iter()
        .find(|p| p.pid == child.id())
        .expect("child listed")
        .clone();
    assert!(target.terminable, "{:?}", target.protected_reason);
    let e = c
        .request(Action::ProcessTerminate {
            pid: target.pid,
            name: target.name.clone(),
            force: true,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::ScopeDenied);
    drop(c);

    agent
        .state
        .store
        .set_scopes(
            &phone.fingerprint,
            &[Scope::DeviceRead, Scope::ProcessTerminate],
        )
        .unwrap();
    let (mut c, _) = Client::connect(&agent, &phone).await;

    // The agent itself is protected.
    let e = c
        .request(Action::ProcessTerminate {
            pid: me.pid,
            name: me.name.clone(),
            force: true,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Forbidden);
    // PID-reuse guard: the name must still match.
    let e = c
        .request(Action::ProcessTerminate {
            pid: target.pid,
            name: "something-else".into(),
            force: true,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);

    // Graceful first. Windows: a windowless console process has no graceful
    // path (fake window adapter returns 0). Linux: SIGTERM ends `sleep`.
    let graceful = c
        .request(Action::ProcessTerminate {
            pid: target.pid,
            name: target.name.clone(),
            force: false,
        })
        .await;
    #[cfg(windows)]
    {
        assert_eq!(graceful.unwrap_err().code, ErrorCode::Unsupported);
        match c
            .request(Action::ProcessTerminate {
                pid: target.pid,
                name: target.name.clone(),
                force: true,
            })
            .await
            .unwrap()
        {
            ActionResult::Terminated(t) => {
                assert!(t.exited);
                assert_eq!(t.method, TerminateMethod::Force);
            }
            other => panic!("{other:?}"),
        }
    }
    #[cfg(not(windows))]
    match graceful.unwrap() {
        ActionResult::Terminated(t) => assert!(t.exited),
        other => panic!("{other:?}"),
    }
    let _ = child.wait();
}

#[tokio::test]
async fn app_list_and_close_are_scoped() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut c = paired_client(&agent, &phone, vec![Scope::DeviceRead]).await;
    match c.request(Action::AppList).await.unwrap() {
        ActionResult::Apps { windows } => assert_eq!(windows[0].title, "notes.txt - Notepad"),
        other => panic!("{other:?}"),
    }
    let e = c
        .request(Action::AppClose {
            window_id: 77,
            pid: 4242,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::ScopeDenied);
    agent
        .state
        .store
        .set_scopes(
            &phone.fingerprint,
            &[Scope::DeviceRead, Scope::ProcessTerminate],
        )
        .unwrap();
    assert_eq!(
        c.request(Action::AppClose {
            window_id: 77,
            pid: 4242
        })
        .await
        .unwrap(),
        ActionResult::Done
    );
    let e = c
        .request(Action::AppClose {
            window_id: 78,
            pid: 4242,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
}

#[tokio::test]
async fn controller_can_unpair_itself() {
    let agent = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    // No scopes at all: unpairing yourself needs none.
    let mut c = paired_client(&agent, &phone, vec![]).await;
    assert_eq!(c.request(Action::Unpair).await.unwrap(), ActionResult::Done);
    match c.recv().await {
        Message::Event {
            event: AgentEvent::Revoked { .. },
        } => {}
        other => panic!("unexpected {other:?}"),
    }
    assert!(agent
        .state
        .store
        .get_controller(&phone.fingerprint)
        .unwrap()
        .is_none());
    let (_, ack) = Client::connect(&agent, &phone).await;
    assert!(!ack.paired);
}

// ------------------------------------------------------------------ audio

use ccontrol_audio_engine::device::{AudioSink, AudioSource, AudioSystem, MemorySink, ToneSource};
use ccontrol_audio_engine::{AudioError, Frame};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

const APP_PID: u32 = 4242;
const APP_SESSION: &str = "session-4242";

/// Fake audio system: one app session (pid 4242) whose mute state is
/// observable; capture is a tone; output is an in-memory sink.
struct FakeAudio {
    mutes: Arc<Mutex<HashMap<String, bool>>>,
    /// Simulate a platform where muting the session also silences capture.
    mute_silences_capture: bool,
}

impl FakeAudio {
    fn new() -> Self {
        Self {
            mutes: Arc::new(Mutex::new(HashMap::from([(
                APP_SESSION.to_string(),
                false,
            )]))),
            mute_silences_capture: false,
        }
    }
    fn muted(&self) -> bool {
        self.mutes.lock().unwrap()[APP_SESSION]
    }
    fn set_muted(&self, m: bool) {
        self.mutes.lock().unwrap().insert(APP_SESSION.into(), m);
    }
}

struct AppCapture {
    tone: ToneSource,
    mutes: Arc<Mutex<HashMap<String, bool>>>,
    silenced_by_mute: bool,
}

impl AudioSource for AppCapture {
    fn kind(&self) -> CaptureKind {
        CaptureKind::Process
    }
    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, AudioError> {
        let f = self.tone.read_frame(stop)?;
        if self.silenced_by_mute && self.mutes.lock().unwrap()[APP_SESSION] {
            return Ok(f.map(|f| vec![0.0; f.len()]));
        }
        Ok(f)
    }
}

impl AudioSystem for FakeAudio {
    fn inventory(&self) -> Result<AudioInventory, AudioError> {
        Ok(AudioInventory {
            endpoints: vec![AudioEndpoint {
                id: "fake".into(),
                name: "Fake speakers".into(),
                is_default: true,
            }],
            sessions: vec![],
            endpoint_capture: true,
            process_capture: true,
            process_capture_reason: None,
            can_play: true,
        })
    }
    fn open_source(&self, src: &RouteSource) -> Result<Box<dyn AudioSource>, AudioError> {
        match src {
            RouteSource::Session { pid: APP_PID, .. } => Ok(Box::new(AppCapture {
                tone: ToneSource::new(0.3),
                mutes: self.mutes.clone(),
                silenced_by_mute: self.mute_silences_capture,
            })),
            RouteSource::Session { pid, .. } => {
                Err(AudioError::Invalid(format!("process {pid} has no audio")))
            }
            _ => Ok(Box::new(ToneSource::new(0.3))),
        }
    }
    fn open_sink(&self, _: Option<&str>) -> Result<Box<dyn AudioSink>, AudioError> {
        Ok(Box::new(MemorySink::new()))
    }
    fn session_mutes(&self, pid: u32) -> Result<Vec<(String, bool)>, AudioError> {
        Ok(if pid == APP_PID {
            self.mutes
                .lock()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        } else {
            vec![]
        })
    }
    fn set_session_mute(&self, id: &str, m: bool) -> Result<bool, AudioError> {
        let mut g = self.mutes.lock().unwrap();
        match g.get_mut(id) {
            Some(v) => {
                *v = m;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    fn silence_local_supported(&self, kind: CaptureKind) -> Result<(), String> {
        if kind == CaptureKind::Process {
            Ok(())
        } else {
            Err("only per-app routes can silence locally".into())
        }
    }
}

fn key() -> String {
    ccontrol_audio_engine::packet::random_key_hex()
}

async fn prepare_sink(c: &mut Client, id: &str, k: &str, lease: u32) -> SinkInfo {
    match c
        .request(Action::SinkPrepare {
            route_id: id.into(),
            key: k.into(),
            endpoint_id: None,
            lease_secs: lease,
        })
        .await
        .unwrap()
    {
        ActionResult::Sink(s) => s,
        other => panic!("{other:?}"),
    }
}

async fn start_route(
    c: &mut Client,
    id: &str,
    k: &str,
    port: u16,
    lease: u32,
    silence: bool,
) -> Result<RouteInfo, ProtocolError> {
    c.request(Action::RouteStart {
        route_id: id.into(),
        source: RouteSource::Session {
            pid: APP_PID,
            app_name: None,
        },
        dest_host: "127.0.0.1".into(),
        dest_port: port,
        key: k.into(),
        lease_secs: lease,
        silence_local: silence,
    })
    .await
    .map(|r| match r {
        ActionResult::Route(r) => r,
        other => panic!("{other:?}"),
    })
}

async fn routes(c: &mut Client) -> (Vec<RouteInfo>, Vec<SinkInfo>) {
    match c.request(Action::RouteList).await.unwrap() {
        ActionResult::Routes { routes, sinks } => (routes, sinks),
        other => panic!("{other:?}"),
    }
}

/// Source agent A, destination agent B, one phone paired to both.
async fn two_agents() -> (Agent, Agent, DeviceIdentity, Client, Client) {
    let a = start_agent().await;
    let b = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let ca = paired_client(&a, &phone, vec![Scope::AudioRoute]).await;
    let cb = paired_client(&b, &phone, vec![Scope::AudioRoute]).await;
    (a, b, phone, ca, cb)
}

#[tokio::test]
async fn audio_route_full_lifecycle_with_true_handoff() {
    let (a, _b, _phone, mut ca, mut cb) = two_agents().await;
    let k = key();
    let sink = prepare_sink(&mut cb, "r1", &k, 30).await;
    assert_eq!(sink.state, SinkState::Waiting);
    assert!(sink.udp_port > 0);

    let r = start_route(&mut ca, "r1", &k, sink.udp_port, 30, true)
        .await
        .unwrap();
    assert_eq!(r.state, RouteState::Active, "{r:?}");
    assert!(r.local_silenced, "{r:?}");
    assert!(
        a.audio.muted(),
        "app muted locally after remote playback was verified"
    );

    tokio::time::sleep(Duration::from_millis(800)).await;
    let (_, sinks) = routes(&mut cb).await;
    assert_eq!(sinks[0].state, SinkState::Playing);
    assert!(sinks[0].stats.frames_played > 30, "{:?}", sinks[0].stats);
    assert!(
        sinks[0].stats.peak > 0.1,
        "decoded tone audible at destination"
    );
    let (rs, _) = routes(&mut ca).await;
    assert!(rs[0].stats.rtt_ms.is_some(), "{:?}", rs[0].stats);
    assert!(rs[0].stats.latency_ms.unwrap() < 250.0, "{:?}", rs[0].stats);

    // Renewal works for the owner.
    assert_eq!(
        ca.request(Action::LeaseRenew {
            route_id: "r1".into()
        })
        .await
        .unwrap(),
        ActionResult::Done
    );

    match ca
        .request(Action::RouteStop {
            route_id: "r1".into(),
        })
        .await
        .unwrap()
    {
        ActionResult::Route(r) => {
            assert_eq!(r.state, RouteState::Idle);
            assert_eq!(r.outcome, Some(RecoveryOutcome::Restored));
        }
        other => panic!("{other:?}"),
    }
    assert!(!a.audio.muted(), "original playback restored");
    // Stop is idempotent.
    match ca
        .request(Action::RouteStop {
            route_id: "r1".into(),
        })
        .await
        .unwrap()
    {
        ActionResult::Route(r) => assert_eq!(r.outcome, Some(RecoveryOutcome::Restored)),
        other => panic!("{other:?}"),
    }
    match cb
        .request(Action::SinkStop {
            route_id: "r1".into(),
        })
        .await
        .unwrap()
    {
        ActionResult::Sink(s) => assert_eq!(s.state, SinkState::Stopped),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn expired_lease_rolls_back_source_and_sink() {
    let (a, b, _phone, mut ca, mut cb) = two_agents().await;
    let k = key();
    let sink = prepare_sink(&mut cb, "r2", &k, 5).await;
    let r = start_route(&mut ca, "r2", &k, sink.udp_port, 5, true)
        .await
        .unwrap();
    assert_eq!(r.state, RouteState::Active);
    assert!(a.audio.muted());
    // The phone "dies": no renewals.
    tokio::time::sleep(Duration::from_millis(6500)).await;
    let (rs, _) = routes(&mut ca).await;
    assert_eq!(rs[0].state, RouteState::Idle, "{:?}", rs[0]);
    assert_eq!(rs[0].outcome, Some(RecoveryOutcome::Restored));
    assert!(
        rs[0].note.as_deref().unwrap_or("").contains("lease"),
        "{:?}",
        rs[0]
    );
    assert!(!a.audio.muted(), "agent restored the app on its own");
    let (_, sinks) = routes(&mut cb).await;
    assert_eq!(sinks[0].state, SinkState::Stopped);
    drop(b);
}

#[tokio::test]
async fn user_change_after_routing_is_not_overwritten() {
    let (a, _b, _phone, mut ca, mut cb) = two_agents().await;
    let k = key();
    let sink = prepare_sink(&mut cb, "r3", &k, 30).await;
    start_route(&mut ca, "r3", &k, sink.udp_port, 30, true)
        .await
        .unwrap();
    assert!(a.audio.muted());
    a.audio.set_muted(false); // the user unmuted the app in the mixer
    match ca
        .request(Action::RouteStop {
            route_id: "r3".into(),
        })
        .await
        .unwrap()
    {
        ActionResult::Route(r) => assert_eq!(r.outcome, Some(RecoveryOutcome::LeftUserChange)),
        other => panic!("{other:?}"),
    }
    assert!(!a.audio.muted());
}

#[tokio::test]
async fn unconfirmed_destination_rolls_back_without_touching_the_app() {
    let (a, _b, _phone, mut ca, _cb) = two_agents().await;
    // Nothing listens on this port: playback can never be confirmed.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let r = start_route(&mut ca, "r4", &key(), port, 30, true)
        .await
        .unwrap();
    assert_eq!(r.state, RouteState::Idle, "{r:?}");
    assert!(r
        .error
        .as_deref()
        .unwrap()
        .contains("did not confirm playback"));
    assert_eq!(r.outcome, Some(RecoveryOutcome::NotNeeded));
    assert!(
        !a.audio.muted(),
        "never silenced before playback was verified"
    );
}

#[tokio::test]
async fn silencing_that_kills_the_capture_is_undone() {
    let a = start_agent_with(Arc::new(FakeAudio {
        mute_silences_capture: true,
        ..FakeAudio::new()
    }))
    .await;
    let b = start_agent().await;
    let phone = DeviceIdentity::generate("phone").unwrap();
    let mut ca = paired_client(&a, &phone, vec![Scope::AudioRoute]).await;
    let mut cb = paired_client(&b, &phone, vec![Scope::AudioRoute]).await;
    let k = key();
    let sink = prepare_sink(&mut cb, "r5", &k, 30).await;
    let r = start_route(&mut ca, "r5", &k, sink.udp_port, 30, true)
        .await
        .unwrap();
    assert_eq!(r.state, RouteState::Active);
    assert!(!r.local_silenced, "{r:?}");
    assert!(
        r.note
            .as_deref()
            .unwrap()
            .contains("local playback was kept"),
        "{r:?}"
    );
    assert!(!a.audio.muted());
}

#[tokio::test]
async fn second_phone_cannot_hijack_or_renew_a_route() {
    let (a, _b, _phone, mut ca, mut cb) = two_agents().await;
    let k = key();
    let sink = prepare_sink(&mut cb, "r6", &k, 30).await;
    start_route(&mut ca, "r6", &k, sink.udp_port, 30, false)
        .await
        .unwrap();
    assert!(!a.audio.muted(), "silence_local=false keeps local playback");
    let other = DeviceIdentity::generate("phone2").unwrap();
    let mut c2 = paired_client(&a, &other, vec![Scope::AudioRoute]).await;
    let e = start_route(&mut c2, "r7", &k, sink.udp_port, 30, false)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy);
    let e = c2
        .request(Action::LeaseRenew {
            route_id: "r6".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Forbidden);
    // Without audio.route nothing is allowed.
    let third = DeviceIdentity::generate("phone3").unwrap();
    let mut c3 = paired_client(&a, &third, vec![Scope::DeviceRead]).await;
    assert_eq!(
        c3.request(Action::RouteList).await.unwrap_err().code,
        ErrorCode::ScopeDenied
    );
}

/// Crash at every state: the journal says where the agent was; a new agent
/// process (new AudioManager over the same database) must restore exactly
/// what was changed and nothing else.
#[test]
fn journal_reconciles_a_crash_at_every_route_state() {
    use ccontrol_agent::audio::AudioManager;
    use ccontrol_persistence::{RouteRecord, Store};
    let states = [
        ("validating", false),
        ("starting_stream", false),
        ("verifying_playback", false),
        ("applying_reversible_source_change", false), // original journaled, mute not yet applied
        ("applying_reversible_source_change", true),  // mute applied, then crash
        ("active", true),
        ("stopping", true),
        ("restoring_source", true),
        ("rolling_back", true),
    ];
    for (state, applied) in states {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("agent.db");
        let audio = Arc::new(FakeAudio::new());
        {
            let store = Store::open(&db).unwrap();
            if applied {
                audio.set_muted(true);
            }
            store
                .journal_route(&RouteRecord {
                    route_id: "crash".into(),
                    role: "source".into(),
                    owner: "phone".into(),
                    state: state.into(),
                    source: serde_json::to_string(&RouteSource::Session {
                        pid: APP_PID,
                        app_name: None,
                    })
                    .unwrap(),
                    dest: "x:1".into(),
                    original: state
                        .starts_with("applying")
                        .then(|| format!("[[\"{APP_SESSION}\",false]]"))
                        .or(applied.then(|| format!("[[\"{APP_SESSION}\",false]]"))),
                    applied: applied.then(|| format!("[\"{APP_SESSION}\"]")),
                    lease_expiry: 0,
                    ..Default::default()
                })
                .unwrap();
            store
                .journal_route(&RouteRecord {
                    route_id: "crash".into(),
                    role: "sink".into(),
                    state: "playing".into(),
                    ..Default::default()
                })
                .unwrap();
        } // "process dies"
        let store = Store::open(&db).unwrap();
        let mgr = AudioManager::new(audio.clone(), &store);
        assert!(
            !audio.muted(),
            "state {state}: app must be audible after recovery"
        );
        let expected = if applied {
            RecoveryOutcome::Restored
        } else {
            RecoveryOutcome::NotNeeded
        };
        let (routes, _) = mgr.list();
        assert_eq!(routes[0].outcome, Some(expected), "state {state}");
        assert_eq!(routes[0].state, RouteState::Idle);
        assert!(
            store.routes(true).unwrap().is_empty(),
            "state {state}: journal fully reconciled"
        );
    }
}

#[test]
fn recovery_leaves_a_user_unmute_alone_and_reports_missing_sessions() {
    use ccontrol_agent::audio::restore;
    let audio = FakeAudio::new();
    let original = vec![(APP_SESSION.to_string(), false)];
    // User already unmuted: CAS keeps their setting.
    assert_eq!(
        restore(&audio, Some(APP_PID), &original, &[APP_SESSION.into()]),
        RecoveryOutcome::LeftUserChange
    );
    // App exited (session gone).
    assert_eq!(
        restore(&audio, Some(APP_PID), &original, &["gone".into()]),
        RecoveryOutcome::Partial
    );
    assert_eq!(
        restore(&audio, Some(APP_PID), &original, &[]),
        RecoveryOutcome::NotNeeded
    );
}
