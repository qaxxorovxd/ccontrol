use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Out-of-band instructions delivered to a live session task.
#[derive(Debug, Clone)]
pub enum SessionControl {
    Kick { reason: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub session_id: u64,
    pub device_id: String,
    pub peer: String,
    pub connected_at: i64,
}

struct Entry {
    info: SessionInfo,
    tx: mpsc::Sender<SessionControl>,
}

/// Live authorized sessions. Multiple controllers (and multiple connections
/// from one controller) may be connected at the same time.
#[derive(Default)]
pub struct SessionRegistry {
    next: AtomicU64,
    inner: Mutex<HashMap<u64, Entry>>,
}

pub struct SessionGuard<'a> {
    reg: &'a SessionRegistry,
    pub id: u64,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        self.reg.lock().remove(&self.id);
    }
}

impl SessionRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn register(
        &self,
        device_id: &str,
        peer: SocketAddr,
    ) -> (SessionGuard<'_>, mpsc::Receiver<SessionControl>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(8);
        let info = SessionInfo {
            session_id: id,
            device_id: device_id.to_owned(),
            peer: peer.to_string(),
            connected_at: ccontrol_persistence::now(),
        };
        self.lock().insert(id, Entry { info, tx });
        (SessionGuard { reg: self, id }, rx)
    }

    pub fn kick(&self, device_id: &str, reason: &str) -> usize {
        let g = self.lock();
        let mut n = 0;
        for e in g.values().filter(|e| e.info.device_id == device_id) {
            let _ = e.tx.try_send(SessionControl::Kick {
                reason: reason.to_owned(),
            });
            n += 1;
        }
        n
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut v: Vec<_> = self.lock().values().map(|e| e.info.clone()).collect();
        v.sort_by_key(|s| s.session_id);
        v
    }

    pub fn is_online(&self, device_id: &str) -> bool {
        self.lock().values().any(|e| e.info.device_id == device_id)
    }
}
