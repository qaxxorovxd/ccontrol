// Release builds on Windows use the GUI subsystem so autostart does not open a
// console window; CLI subcommands re-attach to the parent console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::Context;
use ccontrol_agent::{admin, autostart, config, server, state};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ccontrol-agent", version, about = "ccontrol desktop agent")]
struct Cli {
    /// Override the data directory (identity, trust store, logs).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the agent in the foreground (default).
    Run,
    /// Open the local settings page in the default browser.
    Ui,
    /// Print device identity, paired controllers and capability manifest.
    Status,
    /// Revoke a paired controller by device ID (or unique prefix).
    Revoke { device_id: String },
    /// Start the agent automatically at login (per-user).
    EnableAutostart,
    /// Remove the login autostart registration.
    DisableAutostart,
}

fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn open_in_browser(url: &str) {
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "linux")))]
    let r: std::io::Result<()> = Err(std::io::Error::other("unsupported"));
    if r.is_err() {
        println!("Open {url}");
    }
}

fn init_logging(dir: &std::path::Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let logs = dir.join("logs");
    let _ = std::fs::create_dir_all(&logs);
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("agent")
        .filename_suffix("jsonl")
        .max_log_files(7)
        .build(&logs)
        .expect("log directory");
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("CCONTROL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_writer(file))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
    guard
}

fn main() -> anyhow::Result<()> {
    attach_console();
    let cli = Cli::parse();
    let dir = cli.data_dir.unwrap_or_else(config::default_data_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let cfg = config::AgentConfig::load_or_default(&dir)?;

    match cli.cmd.unwrap_or(Cmd::Run) {
        Cmd::Run => run(dir, cfg),
        Cmd::Ui => {
            let token = admin::load_or_create_token(&dir)?;
            open_in_browser(&admin::admin_url(cfg.admin_port, &token));
            Ok(())
        }
        Cmd::Status => {
            let id = ccontrol_security::DeviceIdentity::load_or_create(&dir, "ccontrol agent")?;
            let store = ccontrol_persistence::Store::open(&dir.join("agent.db"))?;
            let os = ccontrol_capabilities::detect_os();
            let name = cfg.device_name.clone().unwrap_or(os.hostname.clone());
            let m = ccontrol_capabilities::build_manifest(
                &id.fingerprint,
                &name,
                ccontrol_agent::AGENT_VERSION,
                &os,
                &ccontrol_capabilities::probe(),
            );
            println!(
                "device: {name}\nid:     {}\ndata:   {}",
                id.fingerprint,
                dir.display()
            );
            for c in store.list_controllers()? {
                println!(
                    "paired: {} {} [{}]",
                    ccontrol_security::short_fingerprint(&c.device_id),
                    c.name,
                    c.scopes
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            println!("{}", serde_json::to_string_pretty(&m)?);
            Ok(())
        }
        Cmd::Revoke { device_id } => {
            let store = ccontrol_persistence::Store::open(&dir.join("agent.db"))?;
            let matches: Vec<_> = store
                .list_controllers()?
                .into_iter()
                .filter(|c| c.device_id.starts_with(&device_id.to_lowercase()))
                .collect();
            anyhow::ensure!(
                matches.len() == 1,
                "{} controllers match '{device_id}'",
                matches.len()
            );
            store.revoke(&matches[0].device_id)?;
            store.audit(&matches[0].device_id, "pairing.revoke", "cli")?;
            println!(
                "revoked {} (a running agent disconnects it on its next request)",
                matches[0].name
            );
            Ok(())
        }
        Cmd::EnableAutostart => {
            println!("{}", autostart::enable()?);
            Ok(())
        }
        Cmd::DisableAutostart => {
            println!("{}", autostart::disable()?);
            Ok(())
        }
    }
}

fn run(dir: PathBuf, cfg: config::AgentConfig) -> anyhow::Result<()> {
    let _log = init_logging(&dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let identity = ccontrol_security::DeviceIdentity::load_or_create(&dir, "ccontrol agent")?;
        let store = ccontrol_persistence::Store::open(&dir.join("agent.db"))?;
        let token = admin::load_or_create_token(&dir)?;
        let state = state::AppState::new(dir.clone(), cfg.clone(), identity, store, state::Adapters::native(), token.clone());

        let addr = SocketAddr::new(cfg.listen_addr, cfg.listen_port);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding control port {addr} (is another agent running?)"))?;
        let admin_listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.admin_port))
            .await
            .with_context(|| format!("binding settings UI on 127.0.0.1:{}", cfg.admin_port))?;

        tracing::info!(device = %state.device_name, id = %state.identity.fingerprint, %addr, "agent started");
        tracing::info!("settings UI: {}", admin::admin_url(cfg.admin_port, "<token>"));
        eprintln!("Settings UI: {}", admin::admin_url(cfg.admin_port, &token));

        let _mdns = if cfg.mdns_enabled {
            match ccontrol_discovery::Advertiser::start(ccontrol_discovery::AdvertiseParams {
                device_id: &state.identity.fingerprint,
                device_name: &state.device_name,
                os_family: &state.os.family,
                port: cfg.listen_port,
            }) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!(error = %e, "mDNS unavailable; phones must add this computer by address/QR");
                    None
                }
            }
        } else {
            None
        };

        let admin_app = admin::router(state.clone());
        let admin_task = tokio::spawn(async move { axum::serve(admin_listener, admin_app).await });
        let server_task = tokio::spawn(server::serve(state.clone(), listener));

        tokio::select! {
            r = server_task => { r??; }
            r = admin_task => { r??; }
            _ = tokio::signal::ctrl_c() => { tracing::info!("shutdown requested"); }
        }
        // Restore any locally silenced apps before exiting.
        state.audio.shutdown(&state).await;
        Ok(())
    })
}
