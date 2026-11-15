//! Maps typed protocol actions onto native adapters.

use crate::state::AppState;
use ccontrol_protocol::{Action, ActionResult, ErrorCode, ProtocolError, Scope};
use ccontrol_system_control::ControlError;
use std::sync::Arc;

fn map_err(e: ControlError) -> ProtocolError {
    match e {
        ControlError::Unsupported(m) => ProtocolError::new(ErrorCode::Unsupported, m),
        ControlError::Invalid(m) => ProtocolError::new(ErrorCode::InvalidRequest, m),
        ControlError::Platform(m) => ProtocolError::new(ErrorCode::Internal, m),
    }
}

/// Authorize and execute one action for `scopes`.
pub async fn execute(
    state: &Arc<AppState>,
    scopes: &[Scope],
    action: Action,
) -> Result<ActionResult, ProtocolError> {
    action
        .validate()
        .map_err(|m| ProtocolError::new(ErrorCode::InvalidRequest, m))?;
    let need = action.required_scope();
    if !scopes.contains(&need) {
        return Err(ProtocolError::new(
            ErrorCode::ScopeDenied,
            format!(
                "this controller was not granted '{}' on this computer",
                need.as_str()
            ),
        ));
    }
    if let Action::DeviceInfo = action {
        return Ok(ActionResult::Manifest {
            manifest: state.manifest(),
        });
    }
    let adapters = state.adapters.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<ActionResult, ControlError> {
        Ok(match action {
            Action::DeviceInfo => unreachable!(),
            Action::TelemetryGet => ActionResult::Telemetry(adapters.telemetry.sample()),
            Action::VolumeGet => ActionResult::Volume(adapters.volume.get()?),
            Action::VolumeSet { level } => ActionResult::Volume(adapters.volume.set_level(level)?),
            Action::VolumeMute { muted } => ActionResult::Volume(adapters.volume.set_muted(muted)?),
            Action::MediaKey { key } => {
                adapters.media.send(key)?;
                ActionResult::Done
            }
        })
    });
    match tokio::time::timeout(std::time::Duration::from_secs(10), res).await {
        Ok(Ok(r)) => r.map_err(map_err),
        Ok(Err(join)) => Err(ProtocolError::new(
            ErrorCode::Internal,
            format!("adapter crashed: {join}"),
        )),
        Err(_) => Err(ProtocolError::new(
            ErrorCode::Timeout,
            "platform call timed out",
        )),
    }
}
