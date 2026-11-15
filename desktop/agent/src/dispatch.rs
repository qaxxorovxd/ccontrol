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
        ControlError::Forbidden(m) => ProtocolError::new(ErrorCode::Forbidden, m),
    }
}

/// Authorize and execute one action for the controller `caller` holding `scopes`.
pub async fn execute(
    state: &Arc<AppState>,
    caller: &str,
    scopes: &[Scope],
    action: Action,
) -> Result<ActionResult, ProtocolError> {
    action
        .validate()
        .map_err(|m| ProtocolError::new(ErrorCode::InvalidRequest, m))?;
    if let Some(need) = action.required_scope() {
        if !scopes.contains(&need) {
            return Err(ProtocolError::new(
                ErrorCode::ScopeDenied,
                format!(
                    "this controller was not granted '{}' on this computer",
                    need.as_str()
                ),
            ));
        }
    }
    match action {
        Action::DeviceInfo => {
            return Ok(ActionResult::Manifest {
                manifest: state.manifest(),
            })
        }
        Action::Unpair => {
            // Respond first; the session is kicked right after by `revoke`.
            let state = state.clone();
            let caller = caller.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let _ = state.revoke(&caller, "unpaired by the controller");
            });
            return Ok(ActionResult::Done);
        }
        _ => {}
    }
    let adapters = state.adapters.clone();
    let timeout = match action {
        // Graceful stop waits up to 5 s for the app to exit.
        Action::ProcessTerminate { .. } => 15,
        _ => 10,
    };
    let res = tokio::task::spawn_blocking(move || -> Result<ActionResult, ControlError> {
        Ok(match action {
            Action::DeviceInfo | Action::Unpair => unreachable!(),
            Action::TelemetryGet => ActionResult::Telemetry(adapters.telemetry.sample()),
            Action::VolumeGet => ActionResult::Volume(adapters.volume.get()?),
            Action::VolumeSet { level } => ActionResult::Volume(adapters.volume.set_level(level)?),
            Action::VolumeMute { muted } => ActionResult::Volume(adapters.volume.set_muted(muted)?),
            Action::MediaKey { key } => {
                adapters.media.send(key)?;
                ActionResult::Done
            }
            Action::MediaStatus => ActionResult::MediaStatus(adapters.media.status()?),
            Action::ProcessList => ActionResult::Processes {
                processes: adapters.processes.list(),
            },
            Action::ProcessTerminate { pid, name, force } => {
                let windows = &adapters.windows;
                #[cfg(windows)]
                let graceful = |pid: u32| windows.close_all_of(pid);
                #[cfg(not(windows))]
                let graceful = |pid: u32| {
                    let _ = windows;
                    ccontrol_system_control::processes::sigterm(pid)
                };
                ActionResult::Terminated(
                    adapters.processes.terminate(pid, &name, force, &graceful)?,
                )
            }
            Action::AppList => ActionResult::Apps {
                windows: adapters.windows.list()?,
            },
            Action::AppClose { window_id, pid } => {
                adapters.windows.close(window_id, pid)?;
                ActionResult::Done
            }
        })
    });
    match tokio::time::timeout(std::time::Duration::from_secs(timeout), res).await {
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
