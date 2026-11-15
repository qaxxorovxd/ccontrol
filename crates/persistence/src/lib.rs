//! Durable agent state: trusted controllers and an audit log.
//!
//! Notification bodies, credentials and captured media are never written here.

use ccontrol_protocol::Scope;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("corrupt record: {0}")]
    Corrupt(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrustedController {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub scopes: Vec<Scope>,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub ts: i64,
    pub device_id: String,
    pub action: String,
    pub outcome: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA_VERSION: i64 = 1;
/// Keep the audit table bounded.
const AUDIT_MAX_ROWS: i64 = 5000;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS controllers (
                device_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                platform TEXT NOT NULL,
                scopes TEXT NOT NULL,
                paired_at INTEGER NOT NULL,
                last_seen INTEGER
             );
             CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                device_id TEXT NOT NULL,
                action TEXT NOT NULL,
                outcome TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS route_journal (
                route_id TEXT NOT NULL,
                role TEXT NOT NULL,
                owner TEXT NOT NULL,
                state TEXT NOT NULL,
                source TEXT NOT NULL,
                dest TEXT NOT NULL,
                original TEXT,
                applied TEXT,
                lease_expiry INTEGER NOT NULL,
                outcome TEXT,
                note TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (route_id, role)
             );",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-call; SQLite
        // itself keeps the database consistent, so continue.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get_controller(&self, device_id: &str) -> Result<Option<TrustedController>, StoreError> {
        let c = self.conn();
        let row = c
            .query_row(
                "SELECT device_id, name, platform, scopes, paired_at, last_seen
                 FROM controllers WHERE device_id = ?1",
                params![device_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()?;
        row.map(to_controller).transpose()
    }

    pub fn list_controllers(&self) -> Result<Vec<TrustedController>, StoreError> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT device_id, name, platform, scopes, paired_at, last_seen
             FROM controllers ORDER BY paired_at",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.map(|r| to_controller(r?)).collect()
    }

    pub fn upsert_controller(&self, t: &TrustedController) -> Result<(), StoreError> {
        let scopes =
            serde_json::to_string(&t.scopes).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        self.conn().execute(
            "INSERT INTO controllers(device_id, name, platform, scopes, paired_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device_id) DO UPDATE SET
               name = excluded.name, platform = excluded.platform,
               scopes = excluded.scopes, paired_at = excluded.paired_at",
            params![
                t.device_id,
                t.name,
                t.platform,
                scopes,
                t.paired_at,
                t.last_seen
            ],
        )?;
        Ok(())
    }

    pub fn set_scopes(&self, device_id: &str, scopes: &[Scope]) -> Result<bool, StoreError> {
        let s = serde_json::to_string(scopes).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        Ok(self.conn().execute(
            "UPDATE controllers SET scopes = ?2 WHERE device_id = ?1",
            params![device_id, s],
        )? > 0)
    }

    /// Remove trust. Returns true if the controller existed.
    pub fn revoke(&self, device_id: &str) -> Result<bool, StoreError> {
        Ok(self.conn().execute(
            "DELETE FROM controllers WHERE device_id = ?1",
            params![device_id],
        )? > 0)
    }

    pub fn touch(&self, device_id: &str) -> Result<(), StoreError> {
        self.conn().execute(
            "UPDATE controllers SET last_seen = ?2 WHERE device_id = ?1",
            params![device_id, now()],
        )?;
        Ok(())
    }

    pub fn audit(&self, device_id: &str, action: &str, outcome: &str) -> Result<(), StoreError> {
        let c = self.conn();
        c.execute(
            "INSERT INTO audit(ts, device_id, action, outcome) VALUES (?1, ?2, ?3, ?4)",
            params![now(), device_id, action, outcome],
        )?;
        c.execute(
            "DELETE FROM audit WHERE id <= (SELECT MAX(id) FROM audit) - ?1",
            params![AUDIT_MAX_ROWS],
        )?;
        Ok(())
    }

    /// Write-ahead: persist a route's state *before* acting on it.
    pub fn journal_route(&self, r: &RouteRecord) -> Result<(), StoreError> {
        self.conn().execute(
            "INSERT INTO route_journal(route_id, role, owner, state, source, dest, original, applied,
                                       lease_expiry, outcome, note, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(route_id, role) DO UPDATE SET
               owner = excluded.owner, state = excluded.state, source = excluded.source,
               dest = excluded.dest, original = excluded.original, applied = excluded.applied,
               lease_expiry = excluded.lease_expiry, outcome = excluded.outcome,
               note = excluded.note, updated_at = excluded.updated_at",
            params![
                r.route_id, r.role, r.owner, r.state, r.source, r.dest, r.original, r.applied,
                r.lease_expiry, r.outcome, r.note, now()
            ],
        )?;
        Ok(())
    }

    pub fn routes(&self, only_unfinished: bool) -> Result<Vec<RouteRecord>, StoreError> {
        let c = self.conn();
        let sql = if only_unfinished {
            "SELECT route_id, role, owner, state, source, dest, original, applied, lease_expiry, outcome, note
             FROM route_journal WHERE state NOT IN ('idle', 'recovery_required', 'stopped', 'failed')
             ORDER BY updated_at"
        } else {
            "SELECT route_id, role, owner, state, source, dest, original, applied, lease_expiry, outcome, note
             FROM route_journal ORDER BY updated_at DESC LIMIT 200"
        };
        let mut st = c.prepare(sql)?;
        let rows = st.query_map([], |r| {
            Ok(RouteRecord {
                route_id: r.get(0)?,
                role: r.get(1)?,
                owner: r.get(2)?,
                state: r.get(3)?,
                source: r.get(4)?,
                dest: r.get(5)?,
                original: r.get(6)?,
                applied: r.get(7)?,
                lease_expiry: r.get(8)?,
                outcome: r.get(9)?,
                note: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Drop finished journal rows older than `max_age_secs`.
    pub fn prune_routes(&self, max_age_secs: i64) -> Result<usize, StoreError> {
        Ok(self.conn().execute(
            "DELETE FROM route_journal WHERE state IN ('idle', 'stopped', 'failed') AND updated_at < ?1",
            params![now() - max_age_secs],
        )?)
    }

    pub fn recent_audit(&self, limit: u32) -> Result<Vec<AuditEntry>, StoreError> {
        let c = self.conn();
        let mut st = c.prepare(
            "SELECT ts, device_id, action, outcome FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st.query_map(params![limit], |r| {
            Ok(AuditEntry {
                ts: r.get(0)?,
                device_id: r.get(1)?,
                action: r.get(2)?,
                outcome: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn to_controller(
    (device_id, name, platform, scopes, paired_at, last_seen): (
        String,
        String,
        String,
        String,
        i64,
        Option<i64>,
    ),
) -> Result<TrustedController, StoreError> {
    let scopes: Vec<String> =
        serde_json::from_str(&scopes).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    // Unknown scopes from a newer version are dropped, never widened.
    let scopes = scopes.iter().filter_map(|s| Scope::parse(s)).collect();
    Ok(TrustedController {
        device_id,
        name,
        platform,
        scopes,
        paired_at,
        last_seen,
    })
}

/// One row of the write-ahead route journal.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RouteRecord {
    pub route_id: String,
    /// "source" or "sink".
    pub role: String,
    pub owner: String,
    /// snake_case `RouteState` / `SinkState`.
    pub state: String,
    /// JSON `RouteSource`.
    pub source: String,
    pub dest: String,
    /// JSON `[(session_id, muted)]` captured before any local change.
    pub original: Option<String>,
    /// JSON `[session_id]` this agent actually changed.
    pub applied: Option<String>,
    pub lease_expiry: i64,
    pub outcome: Option<String>,
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> TrustedController {
        TrustedController {
            device_id: id.into(),
            name: "Pixel".into(),
            platform: "android".into(),
            scopes: vec![Scope::DeviceRead, Scope::MediaControl],
            paired_at: 100,
            last_seen: None,
        }
    }

    #[test]
    fn trust_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agent.db");
        {
            let s = Store::open(&p).unwrap();
            s.upsert_controller(&sample("abc")).unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(
            s.get_controller("abc").unwrap().unwrap().scopes,
            sample("abc").scopes
        );
    }

    #[test]
    fn route_journal_upserts_and_filters_unfinished() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agent.db");
        {
            let s = Store::open(&p).unwrap();
            let mut r = RouteRecord {
                route_id: "r1".into(),
                role: "source".into(),
                state: "active".into(),
                original: Some("[[\"s\",false]]".into()),
                ..Default::default()
            };
            s.journal_route(&r).unwrap();
            r.route_id = "r2".into();
            r.state = "idle".into();
            s.journal_route(&r).unwrap();
        }
        // Survives a reopen (crash): only the unfinished route is returned.
        let s = Store::open(&p).unwrap();
        let open = s.routes(true).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].route_id, "r1");
        assert_eq!(open[0].original.as_deref(), Some("[[\"s\",false]]"));
        assert_eq!(s.routes(false).unwrap().len(), 2);
    }

    #[test]
    fn revoke_removes_trust() {
        let s = Store::in_memory().unwrap();
        s.upsert_controller(&sample("abc")).unwrap();
        assert!(s.revoke("abc").unwrap());
        assert!(s.get_controller("abc").unwrap().is_none());
        assert!(!s.revoke("abc").unwrap());
    }

    #[test]
    fn scopes_update_and_audit() {
        let s = Store::in_memory().unwrap();
        s.upsert_controller(&sample("abc")).unwrap();
        s.set_scopes("abc", &[Scope::DeviceRead]).unwrap();
        assert_eq!(
            s.get_controller("abc").unwrap().unwrap().scopes,
            vec![Scope::DeviceRead]
        );
        s.audit("abc", "volume.set", "ok").unwrap();
        assert_eq!(s.recent_audit(10).unwrap().len(), 1);
    }
}
