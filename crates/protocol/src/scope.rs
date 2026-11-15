use serde::{Deserialize, Serialize};

/// Independently grantable and revocable permission scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "device.read")]
    DeviceRead,
    #[serde(rename = "telemetry.read")]
    TelemetryRead,
    #[serde(rename = "notifications.read")]
    NotificationsRead,
    #[serde(rename = "media.control")]
    MediaControl,
    #[serde(rename = "audio.route")]
    AudioRoute,
    #[serde(rename = "input.control")]
    InputControl,
    #[serde(rename = "files.read")]
    FilesRead,
    #[serde(rename = "files.write")]
    FilesWrite,
    #[serde(rename = "process.terminate")]
    ProcessTerminate,
    #[serde(rename = "power.control")]
    PowerControl,
    #[serde(rename = "commands.run")]
    CommandsRun,
    #[serde(rename = "admin.action")]
    AdminAction,
}

impl Scope {
    pub const ALL: [Scope; 12] = [
        Scope::DeviceRead,
        Scope::TelemetryRead,
        Scope::NotificationsRead,
        Scope::MediaControl,
        Scope::AudioRoute,
        Scope::InputControl,
        Scope::FilesRead,
        Scope::FilesWrite,
        Scope::ProcessTerminate,
        Scope::PowerControl,
        Scope::CommandsRun,
        Scope::AdminAction,
    ];

    /// Scopes a desktop owner can grant with one click on a pairing request.
    /// High-risk scopes must be ticked explicitly.
    pub const LOW_RISK_DEFAULT: [Scope; 3] =
        [Scope::DeviceRead, Scope::TelemetryRead, Scope::MediaControl];

    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::DeviceRead => "device.read",
            Scope::TelemetryRead => "telemetry.read",
            Scope::NotificationsRead => "notifications.read",
            Scope::MediaControl => "media.control",
            Scope::AudioRoute => "audio.route",
            Scope::InputControl => "input.control",
            Scope::FilesRead => "files.read",
            Scope::FilesWrite => "files.write",
            Scope::ProcessTerminate => "process.terminate",
            Scope::PowerControl => "power.control",
            Scope::CommandsRun => "commands.run",
            Scope::AdminAction => "admin.action",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|sc| sc.as_str() == s)
    }

    pub fn is_high_risk(&self) -> bool {
        matches!(
            self,
            Scope::InputControl
                | Scope::FilesWrite
                | Scope::ProcessTerminate
                | Scope::PowerControl
                | Scope::CommandsRun
                | Scope::AdminAction
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_round_trip() {
        for s in Scope::ALL {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
            assert_eq!(serde_json::from_str::<Scope>(&json).unwrap(), s);
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
    }
}
