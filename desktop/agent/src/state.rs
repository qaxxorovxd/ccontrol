use crate::audio::AudioManager;
use crate::config::AgentConfig;
use crate::pairing::PairingManager;
use crate::sessions::SessionRegistry;
use ccontrol_audio_engine::device::AudioSystem;
use ccontrol_capabilities::{build_manifest, OsInfo, Probes};
use ccontrol_persistence::Store;
use ccontrol_protocol::CapabilityManifest;
use ccontrol_security::DeviceIdentity;
use ccontrol_system_control::{
    MediaControl, ProcessManager, TelemetrySampler, VolumeControl, WindowControl,
};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Platform adapters, injectable for tests.
pub struct Adapters {
    pub volume: Box<dyn VolumeControl>,
    pub media: Box<dyn MediaControl>,
    pub telemetry: TelemetrySampler,
    pub processes: ProcessManager,
    pub windows: Box<dyn WindowControl>,
    pub audio: Arc<dyn AudioSystem>,
}

impl Adapters {
    pub fn native() -> Self {
        Self {
            volume: ccontrol_system_control::volume(),
            media: ccontrol_system_control::media(),
            telemetry: TelemetrySampler::new(),
            processes: ProcessManager::new(),
            windows: ccontrol_system_control::windows_adapter(),
            audio: ccontrol_audio_engine::device::native(),
        }
    }
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub config: AgentConfig,
    pub device_name: String,
    pub identity: DeviceIdentity,
    pub store: Store,
    pub os: OsInfo,
    pub manifest: RwLock<CapabilityManifest>,
    pub pairing: PairingManager,
    pub sessions: SessionRegistry,
    pub adapters: Arc<Adapters>,
    pub audio: AudioManager,
    pub admin_token: String,
}

impl AppState {
    pub fn new(
        data_dir: PathBuf,
        config: AgentConfig,
        identity: DeviceIdentity,
        store: Store,
        adapters: Adapters,
        admin_token: String,
    ) -> Arc<Self> {
        let os = ccontrol_capabilities::detect_os();
        let device_name = config
            .device_name
            .clone()
            .unwrap_or_else(|| os.hostname.clone());
        let probes = Probes {
            volume: Some(adapters.volume.get().map(|_| ()).map_err(|e| e.to_string())),
            media: Some(
                adapters
                    .media
                    .status()
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            ),
            windows: Some(
                adapters
                    .windows
                    .list()
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            ),
            audio: Some(
                adapters
                    .audio
                    .inventory()
                    .map(|i| ccontrol_capabilities::AudioProbe {
                        can_play: i.can_play,
                        endpoint_capture: i.endpoint_capture,
                        process_capture: i.process_capture,
                        process_capture_reason: i.process_capture_reason,
                    })
                    .map_err(|e| e.to_string()),
            ),
        };
        // Reconciles the route journal (crash recovery) before anything else.
        let audio = AudioManager::new(adapters.audio.clone(), &store);
        let manifest = build_manifest(
            &identity.fingerprint,
            &device_name,
            crate::AGENT_VERSION,
            &os,
            &probes,
        );
        Arc::new(Self {
            data_dir,
            config,
            device_name,
            identity,
            store,
            os,
            manifest: RwLock::new(manifest),
            pairing: PairingManager::default(),
            sessions: SessionRegistry::default(),
            adapters: Arc::new(adapters),
            audio,
            admin_token,
        })
    }

    pub fn manifest(&self) -> CapabilityManifest {
        self.manifest
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Revoke trust and immediately disconnect any live sessions.
    pub fn revoke(&self, device_id: &str, reason: &str) -> anyhow::Result<bool> {
        let existed = self.store.revoke(device_id)?;
        self.sessions.kick(device_id, reason);
        let _ = self.store.audit(device_id, "pairing.revoke", reason);
        Ok(existed)
    }
}
