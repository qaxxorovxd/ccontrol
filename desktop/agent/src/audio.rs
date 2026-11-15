//! Audio routes (source side) and sinks (destination side).
//!
//! The agent — not the phone — owns route state. Every transition is written
//! to the SQLite route journal *before* the agent acts on it, so a crash at
//! any point is recoverable on the next start (`reconcile`). Local changes
//! (muting the app's sessions for a true handoff) are restored with
//! compare-and-swap semantics: a setting the user changed after we applied
//! ours is left alone.

use crate::state::AppState;
use ccontrol_audio_engine::device::AudioSystem;
use ccontrol_audio_engine::packet::StreamKey;
use ccontrol_audio_engine::{ReceiverHandle, SenderHandle};
use ccontrol_persistence::{RouteRecord, Store};
use ccontrol_protocol::{
    CaptureKind, ErrorCode, ProtocolError, RecoveryOutcome, RouteInfo, RouteSource, RouteState,
    SinkInfo, SinkState, StreamStats,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
/// Receiver reports older than this mean the destination went away.
const DEST_SILENT_LIMIT: Duration = Duration::from_secs(8);

fn err(code: ErrorCode, m: impl Into<String>) -> ProtocolError {
    ProtocolError::new(code, m)
}

fn state_str(s: RouteState) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn outcome_str(o: RecoveryOutcome) -> String {
    serde_json::to_value(o)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

struct SourceRoute {
    info: RouteInfo,
    pid: Option<u32>,
    sender: Option<SenderHandle>,
    original: Vec<(String, bool)>,
    applied: Vec<String>,
    lease: Duration,
    lease_expiry: Instant,
    started: Instant,
}

struct SinkEntry {
    info: SinkInfo,
    receiver: Option<ReceiverHandle>,
    lease: Duration,
    lease_expiry: Instant,
}

pub struct AudioManager {
    pub system: Arc<dyn AudioSystem>,
    routes: Mutex<HashMap<String, SourceRoute>>,
    sinks: Mutex<HashMap<String, SinkEntry>>,
    monitor_started: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn unix_after(d: Duration) -> i64 {
    ccontrol_persistence::now() + d.as_secs() as i64
}

/// Restore what a route changed, without overriding later user changes.
pub fn restore(
    system: &dyn AudioSystem,
    pid: Option<u32>,
    original: &[(String, bool)],
    applied: &[String],
) -> RecoveryOutcome {
    if applied.is_empty() {
        return RecoveryOutcome::NotNeeded;
    }
    let Some(pid) = pid else {
        return RecoveryOutcome::ManualActionRequired;
    };
    let current = match system.session_mutes(pid) {
        Ok(c) => c,
        Err(_) => return RecoveryOutcome::ManualActionRequired,
    };
    let (mut restored, mut user, mut missing, mut failed) = (0, 0, 0, 0);
    for id in applied {
        let orig = original
            .iter()
            .find(|(s, _)| s == id)
            .map(|(_, m)| *m)
            .unwrap_or(false);
        match current.iter().find(|(s, _)| s == id) {
            None => missing += 1,
            // Already back at the original value: the user undid our change.
            Some((_, cur)) if *cur == orig => user += 1,
            Some(_) => match system.set_session_mute(id, orig) {
                Ok(true) => restored += 1,
                Ok(false) => missing += 1,
                Err(_) => failed += 1,
            },
        }
    }
    if failed > 0 {
        RecoveryOutcome::ManualActionRequired
    } else if missing > 0 {
        RecoveryOutcome::Partial
    } else if restored == 0 && user > 0 {
        RecoveryOutcome::LeftUserChange
    } else {
        RecoveryOutcome::Restored
    }
}

/// Startup recovery: finish every route the journal shows as unfinished.
/// Returns the reconciled routes (shown to controllers as recent history).
pub fn reconcile(system: &dyn AudioSystem, store: &Store) -> Vec<RouteInfo> {
    let mut out = vec![];
    for mut r in store.routes(true).unwrap_or_default() {
        if r.role == "sink" {
            r.state = "stopped".into();
            r.note = Some("agent restarted".into());
            let _ = store.journal_route(&r);
            continue;
        }
        let source: RouteSource = serde_json::from_str(&r.source).unwrap_or(RouteSource::Endpoint);
        let pid = match &source {
            RouteSource::Session { pid, .. } => Some(*pid),
            _ => None,
        };
        let original: Vec<(String, bool)> = r
            .original
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let applied: Vec<String> = r
            .applied
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let outcome = restore(system, pid, &original, &applied);
        let state = if outcome == RecoveryOutcome::ManualActionRequired {
            RouteState::RecoveryRequired
        } else {
            RouteState::Idle
        };
        let interrupted = r.state.clone();
        r.state = state_str(state);
        r.outcome = Some(outcome_str(outcome));
        r.note = Some(format!(
            "recovered after the agent stopped during '{interrupted}'"
        ));
        let _ = store.journal_route(&r);
        tracing::warn!(route = %r.route_id, %interrupted, ?outcome, "reconciled unfinished audio route");
        out.push(RouteInfo {
            route_id: r.route_id.clone(),
            state,
            capture: match source {
                RouteSource::Session { .. } => CaptureKind::Process,
                RouteSource::Endpoint => CaptureKind::Endpoint,
                RouteSource::TestTone => CaptureKind::TestTone,
            },
            source,
            dest: r.dest.clone(),
            owner: r.owner.clone(),
            silence_local_requested: !original.is_empty(),
            local_silenced: false,
            lease_remaining_ms: 0,
            outcome: Some(outcome),
            note: r.note.clone(),
            error: None,
            stats: StreamStats::default(),
        });
    }
    let _ = store.prune_routes(7 * 24 * 3600);
    out
}

impl AudioManager {
    /// Creates the manager and immediately reconciles the journal.
    pub fn new(system: Arc<dyn AudioSystem>, store: &Store) -> Self {
        let recovered = reconcile(system.as_ref(), store);
        let now = Instant::now();
        let routes = recovered
            .into_iter()
            .map(|info| {
                (
                    info.route_id.clone(),
                    SourceRoute {
                        pid: None,
                        sender: None,
                        original: vec![],
                        applied: vec![],
                        lease: Duration::ZERO,
                        lease_expiry: now,
                        started: now,
                        info,
                    },
                )
            })
            .collect();
        Self {
            system,
            routes: Mutex::new(routes),
            sinks: Mutex::default(),
            monitor_started: AtomicBool::new(false),
        }
    }

    fn journal(&self, store: &Store, r: &SourceRoute) {
        let rec = RouteRecord {
            route_id: r.info.route_id.clone(),
            role: "source".into(),
            owner: r.info.owner.clone(),
            state: state_str(r.info.state),
            source: serde_json::to_string(&r.info.source).unwrap_or_default(),
            dest: r.info.dest.clone(),
            original: (!r.original.is_empty())
                .then(|| serde_json::to_string(&r.original).unwrap_or_default()),
            applied: (!r.applied.is_empty())
                .then(|| serde_json::to_string(&r.applied).unwrap_or_default()),
            lease_expiry: unix_after(r.lease_expiry.saturating_duration_since(Instant::now())),
            outcome: r.info.outcome.map(outcome_str),
            note: r.info.note.clone(),
        };
        if let Err(e) = store.journal_route(&rec) {
            tracing::error!(error = %e, "route journal write failed");
        }
    }

    /// Set state and journal it (write-ahead: called before acting).
    fn transition(&self, store: &Store, id: &str, s: RouteState) {
        let mut g = lock(&self.routes);
        if let Some(r) = g.get_mut(id) {
            r.info.state = s;
            self.journal(store, r);
        }
    }

    fn info_of(r: &SourceRoute) -> RouteInfo {
        let mut i = r.info.clone();
        if let Some(s) = &r.sender {
            i.stats = s.stats();
        }
        i.lease_remaining_ms = if i.state.is_terminal() {
            0
        } else {
            r.lease_expiry
                .saturating_duration_since(Instant::now())
                .as_millis() as u64
        };
        i
    }

    fn ensure_monitor(&self, state: &Arc<AppState>) {
        if self.monitor_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                state.audio.tick(&state).await;
            }
        });
    }

    // ------------------------------------------------------------ source

    #[allow(clippy::too_many_arguments)]
    pub async fn start_route(
        &self,
        state: &Arc<AppState>,
        owner: &str,
        route_id: String,
        source: RouteSource,
        dest_host: String,
        dest_port: u16,
        key: String,
        lease_secs: u32,
        silence_local: bool,
    ) -> Result<RouteInfo, ProtocolError> {
        self.ensure_monitor(state);
        let store = &state.store;
        let capture = match &source {
            RouteSource::Session { .. } => CaptureKind::Process,
            RouteSource::Endpoint => CaptureKind::Endpoint,
            RouteSource::TestTone => CaptureKind::TestTone,
        };
        let pid = match &source {
            RouteSource::Session { pid, .. } => Some(*pid),
            _ => None,
        };
        {
            let mut g = lock(&self.routes);
            if let Some(r) = g.get(&route_id) {
                if !r.info.state.is_terminal() {
                    return Err(err(ErrorCode::InvalidRequest, "route id already in use"));
                }
                g.remove(&route_id);
            }
            // Arbitration: one live route per source (test tones excepted).
            if capture != CaptureKind::TestTone {
                if let Some(other) = g
                    .values()
                    .find(|r| !r.info.state.is_terminal() && r.info.source == source)
                {
                    return Err(if other.info.owner == owner {
                        err(
                            ErrorCode::InvalidRequest,
                            "this source is already being routed; stop that route first",
                        )
                    } else {
                        err(
                            ErrorCode::Busy,
                            "another phone is already routing this source",
                        )
                    });
                }
            }
            let lease = Duration::from_secs(lease_secs as u64);
            let r = SourceRoute {
                info: RouteInfo {
                    route_id: route_id.clone(),
                    state: RouteState::Validating,
                    source: source.clone(),
                    capture,
                    dest: format!("{dest_host}:{dest_port}"),
                    owner: owner.to_owned(),
                    silence_local_requested: silence_local,
                    local_silenced: false,
                    lease_remaining_ms: lease.as_millis() as u64,
                    outcome: None,
                    note: None,
                    error: None,
                    stats: StreamStats::default(),
                },
                pid,
                sender: None,
                original: vec![],
                applied: vec![],
                lease,
                lease_expiry: Instant::now() + lease,
                started: Instant::now(),
            };
            self.journal(store, &r);
            g.insert(route_id.clone(), r);
        }

        // VALIDATING: resolve destination, open capture.
        let sys = self.system.clone();
        let src2 = source.clone();
        let target = format!("{dest_host}:{dest_port}");
        let rid = route_id.clone();
        let opened = tokio::task::spawn_blocking(move || {
            use std::net::ToSocketAddrs;
            let addr = target
                .to_socket_addrs()
                .map_err(|e| format!("cannot resolve destination {target}: {e}"))?
                .next()
                .ok_or_else(|| format!("no address for {target}"))?;
            let key = StreamKey::new(&rid, &key).map_err(|e| e.to_string())?;
            let src = sys.open_source(&src2).map_err(|e| e.to_string())?;
            Ok::<_, String>((addr, src, Arc::new(key)))
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
        let (addr, src, key) = match opened {
            Ok(v) => v,
            Err(e) => return Ok(self.fail(store, &route_id, e).await),
        };

        // STARTING_STREAM
        self.transition(store, &route_id, RouteState::StartingStream);
        let sender =
            match tokio::task::spawn_blocking(move || SenderHandle::start(src, addr, key)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Ok(self.fail(store, &route_id, e.to_string()).await),
                Err(e) => return Ok(self.fail(store, &route_id, e.to_string()).await),
            };
        if let Some(r) = lock(&self.routes).get_mut(&route_id) {
            r.sender = Some(sender);
        }

        // VERIFYING_PLAYBACK: wait for the destination's receiver report.
        self.transition(store, &route_id, RouteState::VerifyingPlayback);
        let deadline = Instant::now() + VERIFY_TIMEOUT;
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let (played, error) = {
                let g = lock(&self.routes);
                let Some(r) = g.get(&route_id) else {
                    return Err(err(ErrorCode::Internal, "route vanished"));
                };
                if r.info.state.is_terminal() {
                    return Ok(Self::info_of(r)); // stopped meanwhile
                }
                let s = r.sender.as_ref().unwrap();
                (
                    s.receiver_report()
                        .map(|rep| rep.frames_played > 0)
                        .unwrap_or(false),
                    s.error(),
                )
            };
            if let Some(e) = error {
                return Ok(self
                    .fail(store, &route_id, format!("capture failed: {e}"))
                    .await);
            }
            if played {
                break;
            }
            if Instant::now() >= deadline {
                return Ok(self
                    .fail(store, &route_id, "The destination did not confirm playback (is it prepared and reachable over UDP?)".into())
                    .await);
            }
        }

        // APPLYING_REVERSIBLE_SOURCE_CHANGE (optional true handoff).
        if silence_local {
            match self.system.silence_local_supported(capture) {
                Err(reason) => {
                    if let Some(r) = lock(&self.routes).get_mut(&route_id) {
                        r.info.note = Some(reason);
                    }
                }
                Ok(()) => self.apply_silence(store, &route_id, pid.unwrap_or(0)).await,
            }
        }

        // ACTIVE
        self.transition(store, &route_id, RouteState::Active);
        let g = lock(&self.routes);
        Ok(Self::info_of(g.get(&route_id).unwrap()))
    }

    async fn apply_silence(&self, store: &Store, route_id: &str, pid: u32) {
        let sys = self.system.clone();
        let original = tokio::task::spawn_blocking(move || sys.session_mutes(pid))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        {
            // Journal the original state *before* touching anything.
            let mut g = lock(&self.routes);
            let Some(r) = g.get_mut(route_id) else { return };
            r.original = original.clone();
            r.info.state = RouteState::ApplyingReversibleSourceChange;
            self.journal(store, r);
        }
        let before = self.capture_level(route_id).await; // ~300 ms window before muting
        for (id, muted) in original.iter().filter(|(_, m)| !m) {
            let _ = muted;
            let sys = self.system.clone();
            let id2 = id.clone();
            if let Ok(Ok(true)) =
                tokio::task::spawn_blocking(move || sys.set_session_mute(&id2, true)).await
            {
                let mut g = lock(&self.routes);
                if let Some(r) = g.get_mut(route_id) {
                    r.applied.push(id.clone());
                    self.journal(store, r);
                }
            }
        }
        // Let the change propagate, discard that transition window, then measure.
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.capture_level(route_id).await;
        let after = self.capture_level(route_id).await;
        let (applied, orig) = {
            let g = lock(&self.routes);
            let r = g.get(route_id).unwrap();
            (r.applied.clone(), r.original.clone())
        };
        // Safety check: if muting locally also silenced what we capture, undo it.
        if before > 0.02 && after < 0.002 {
            let sys = self.system.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                restore(sys.as_ref(), Some(pid), &orig, &applied)
            })
            .await
            .unwrap_or(RecoveryOutcome::ManualActionRequired);
            let mut g = lock(&self.routes);
            if let Some(r) = g.get_mut(route_id) {
                r.applied.clear();
                r.info.local_silenced = false;
                r.info.note = Some(format!(
                    "Muting the app locally also silenced its capture on this system, so local playback was kept ({outcome:?})."
                ));
                self.journal(store, r);
            }
        } else if let Some(r) = lock(&self.routes).get_mut(route_id) {
            r.info.local_silenced = !r.applied.is_empty();
        }
    }

    /// Highest capture level over the next ~300 ms.
    async fn capture_level(&self, route_id: &str) -> f32 {
        let take = || {
            lock(&self.routes)
                .get(route_id)
                .and_then(|r| r.sender.as_ref().map(|s| s.capture_peak()))
                .unwrap_or(0.0)
        };
        take(); // reset the window
        tokio::time::sleep(Duration::from_millis(300)).await;
        take()
    }

    /// ROLLING_BACK -> IDLE with an error.
    async fn fail(&self, store: &Store, route_id: &str, error: String) -> RouteInfo {
        self.transition(store, route_id, RouteState::RollingBack);
        self.finish(store, route_id, Some(error), None).await
    }

    /// Stop streaming, restore local changes, reach a terminal state. Idempotent.
    async fn finish(
        &self,
        store: &Store,
        route_id: &str,
        error: Option<String>,
        note: Option<String>,
    ) -> RouteInfo {
        let (sender, pid, original, applied) = {
            let mut g = lock(&self.routes);
            let Some(r) = g.get_mut(route_id) else {
                return RouteInfo {
                    route_id: route_id.into(),
                    state: RouteState::Idle,
                    source: RouteSource::Endpoint,
                    capture: CaptureKind::Endpoint,
                    dest: String::new(),
                    owner: String::new(),
                    silence_local_requested: false,
                    local_silenced: false,
                    lease_remaining_ms: 0,
                    outcome: None,
                    note: None,
                    error: Some("unknown route".into()),
                    stats: StreamStats::default(),
                };
            };
            if r.info.state.is_terminal() {
                return Self::info_of(r);
            }
            if r.info.state != RouteState::RollingBack {
                r.info.state = RouteState::Stopping;
                self.journal(store, r);
            }
            if let Some(s) = &r.sender {
                r.info.stats = s.stats();
            }
            (
                r.sender.take(),
                r.pid,
                r.original.clone(),
                r.applied.clone(),
            )
        };
        if let Some(s) = sender {
            let _ = tokio::task::spawn_blocking(move || s.stop()).await;
        }
        self.transition(store, route_id, RouteState::RestoringSource);
        let sys = self.system.clone();
        let outcome =
            tokio::task::spawn_blocking(move || restore(sys.as_ref(), pid, &original, &applied))
                .await
                .unwrap_or(RecoveryOutcome::ManualActionRequired);
        let mut g = lock(&self.routes);
        let r = g.get_mut(route_id).unwrap();
        r.info.outcome = Some(outcome);
        r.info.local_silenced = false;
        r.info.state = if outcome == RecoveryOutcome::ManualActionRequired {
            RouteState::RecoveryRequired
        } else {
            RouteState::Idle
        };
        if error.is_some() {
            r.info.error = error;
        }
        if let Some(n) = note {
            r.info.note = Some(n);
        }
        if outcome == RecoveryOutcome::ManualActionRequired {
            r.info.note = Some("Could not restore the app's sound automatically. Unmute it in the Windows volume mixer.".into());
        }
        self.journal(store, r);
        tracing::info!(route = route_id, ?outcome, state = ?r.info.state, "audio route finished");
        Self::info_of(r)
    }

    pub async fn stop_route(
        &self,
        state: &Arc<AppState>,
        route_id: &str,
        by: &str,
    ) -> Result<RouteInfo, ProtocolError> {
        if !lock(&self.routes).contains_key(route_id) {
            return Err(err(ErrorCode::InvalidRequest, "unknown route"));
        }
        Ok(self
            .finish(
                &state.store,
                route_id,
                None,
                Some(format!("stopped by {by}")),
            )
            .await)
    }

    // ------------------------------------------------------------ sink

    pub async fn prepare_sink(
        &self,
        state: &Arc<AppState>,
        owner: &str,
        route_id: String,
        key: String,
        endpoint_id: Option<String>,
        lease_secs: u32,
    ) -> Result<SinkInfo, ProtocolError> {
        self.ensure_monitor(state);
        if lock(&self.sinks)
            .get(&route_id)
            .map(|s| s.receiver.is_some())
            .unwrap_or(false)
        {
            return Err(err(ErrorCode::InvalidRequest, "sink already prepared"));
        }
        let sys = self.system.clone();
        let rid = route_id.clone();
        let started = tokio::task::spawn_blocking(move || {
            let sink = sys.open_sink(endpoint_id.as_deref())?;
            let key = StreamKey::new(&rid, &key)?;
            ReceiverHandle::start(Arc::new(key), sink)
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| match e {
            ccontrol_audio_engine::AudioError::Unsupported(m) => err(ErrorCode::Unsupported, m),
            ccontrol_audio_engine::AudioError::Invalid(m) => err(ErrorCode::InvalidRequest, m),
            other => err(ErrorCode::Internal, other.to_string()),
        })?;
        let lease = Duration::from_secs(lease_secs as u64);
        let info = SinkInfo {
            route_id: route_id.clone(),
            state: SinkState::Waiting,
            udp_port: started.port(),
            endpoint_name: started.sink_name().to_owned(),
            owner: owner.to_owned(),
            lease_remaining_ms: lease.as_millis() as u64,
            error: None,
            stats: StreamStats::default(),
        };
        let _ = state.store.journal_route(&RouteRecord {
            route_id: route_id.clone(),
            role: "sink".into(),
            owner: owner.into(),
            state: "waiting".into(),
            source: "{}".into(),
            dest: info.endpoint_name.clone(),
            lease_expiry: unix_after(lease),
            ..Default::default()
        });
        lock(&self.sinks).insert(
            route_id,
            SinkEntry {
                info: info.clone(),
                receiver: Some(started),
                lease,
                lease_expiry: Instant::now() + lease,
            },
        );
        Ok(info)
    }

    fn sink_info(e: &SinkEntry) -> SinkInfo {
        let mut i = e.info.clone();
        if let Some(r) = &e.receiver {
            i.stats = r.stats();
            if i.state == SinkState::Waiting && r.playing() {
                i.state = SinkState::Playing;
            }
        }
        i.lease_remaining_ms = if e.receiver.is_some() {
            e.lease_expiry
                .saturating_duration_since(Instant::now())
                .as_millis() as u64
        } else {
            0
        };
        i
    }

    pub async fn stop_sink(
        &self,
        state: &Arc<AppState>,
        route_id: &str,
        reason: Option<String>,
    ) -> Result<SinkInfo, ProtocolError> {
        let receiver = {
            let mut g = lock(&self.sinks);
            let e = g
                .get_mut(route_id)
                .ok_or_else(|| err(ErrorCode::InvalidRequest, "unknown sink"))?;
            if let Some(r) = &e.receiver {
                e.info.stats = r.stats();
            }
            e.info.state = if e.info.error.is_some() {
                SinkState::Failed
            } else {
                SinkState::Stopped
            };
            if reason.is_some() {
                e.info.error = e.info.error.clone().or(reason);
            }
            e.receiver.take()
        };
        if let Some(r) = receiver {
            let _ = tokio::task::spawn_blocking(move || r.stop()).await;
        }
        let g = lock(&self.sinks);
        let e = g.get(route_id).unwrap();
        let _ = state.store.journal_route(&RouteRecord {
            route_id: route_id.into(),
            role: "sink".into(),
            owner: e.info.owner.clone(),
            state: if e.info.state == SinkState::Failed {
                "failed".into()
            } else {
                "stopped".into()
            },
            source: "{}".into(),
            dest: e.info.endpoint_name.clone(),
            note: e.info.error.clone(),
            ..Default::default()
        });
        Ok(Self::sink_info(e))
    }

    // ------------------------------------------------------------ common

    pub fn renew(&self, owner: &str, route_id: &str) -> Result<(), ProtocolError> {
        let mut found = false;
        if let Some(r) = lock(&self.routes).get_mut(route_id) {
            if r.info.owner != owner {
                return Err(err(ErrorCode::Forbidden, "route is owned by another phone"));
            }
            if !r.info.state.is_terminal() {
                r.lease_expiry = Instant::now() + r.lease;
                found = true;
            }
        }
        if let Some(s) = lock(&self.sinks).get_mut(route_id) {
            if s.info.owner != owner {
                return Err(err(ErrorCode::Forbidden, "sink is owned by another phone"));
            }
            if s.receiver.is_some() {
                s.lease_expiry = Instant::now() + s.lease;
                found = true;
            }
        }
        if found {
            Ok(())
        } else {
            Err(err(
                ErrorCode::InvalidRequest,
                "no live route or sink with that id",
            ))
        }
    }

    pub fn list(&self) -> (Vec<RouteInfo>, Vec<SinkInfo>) {
        let mut routes: Vec<_> = lock(&self.routes)
            .values()
            .map(|r| (r.started, Self::info_of(r)))
            .collect();
        routes.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        let sinks = lock(&self.sinks).values().map(Self::sink_info).collect();
        (routes.into_iter().map(|(_, i)| i).collect(), sinks)
    }

    /// Periodic supervision: leases, dead captures, silent destinations.
    async fn tick(&self, state: &Arc<AppState>) {
        let now = Instant::now();
        let mut to_finish: Vec<(String, Option<String>, Option<String>)> = vec![];
        for (id, r) in lock(&self.routes).iter() {
            if r.info.state != RouteState::Active {
                continue;
            }
            if now >= r.lease_expiry {
                to_finish.push((
                    id.clone(),
                    None,
                    Some(
                        "lease expired: the phone stopped renewing (closed or out of range)".into(),
                    ),
                ));
            } else if let Some(s) = &r.sender {
                if let Some(e) = s
                    .error()
                    .or_else(|| (!s.is_running()).then(|| "capture ended".into()))
                {
                    to_finish.push((id.clone(), Some(format!("source stopped: {e}")), None));
                } else if s.receiver_report().is_none()
                    && now.duration_since(r.started) > DEST_SILENT_LIMIT
                {
                    to_finish.push((
                        id.clone(),
                        Some("destination stopped responding".into()),
                        None,
                    ));
                }
            }
        }
        for (id, e, n) in to_finish {
            if e.is_some() {
                self.transition(&state.store, &id, RouteState::RollingBack);
            }
            self.finish(&state.store, &id, e, n).await;
        }
        let mut sinks_to_stop = vec![];
        for (id, s) in lock(&self.sinks).iter_mut() {
            let Some(r) = &s.receiver else { continue };
            if now >= s.lease_expiry {
                sinks_to_stop.push((id.clone(), "lease expired".to_string()));
            } else if let Some(e) = r.error() {
                s.info.error = Some(e.clone());
                sinks_to_stop.push((id.clone(), e));
            }
        }
        for (id, reason) in sinks_to_stop {
            let _ = self.stop_sink(state, &id, Some(reason)).await;
        }
    }

    /// Graceful agent shutdown: finish every live route (restoring sources).
    pub async fn shutdown(&self, state: &Arc<AppState>) {
        let ids: Vec<String> = lock(&self.routes)
            .iter()
            .filter(|(_, r)| !r.info.state.is_terminal())
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            self.finish(&state.store, &id, None, Some("agent shutting down".into()))
                .await;
        }
        let sinks: Vec<String> = lock(&self.sinks)
            .iter()
            .filter(|(_, s)| s.receiver.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for id in sinks {
            let _ = self
                .stop_sink(state, &id, Some("agent shutting down".into()))
                .await;
        }
    }
}
