//! Developer controller CLI: exercises the same protocol as the Android app.
//!
//! ```text
//! ccontrol-ctl --state ./ctl pair 192.168.1.20:47800          # prints SAS, waits for approval
//! ccontrol-ctl --state ./ctl call 192.168.1.20:47800 volume.get
//! ccontrol-ctl --state ./ctl call 192.168.1.20:47800 volume.set 0.4
//! ```

use anyhow::{bail, Context};
use ccontrol_protocol::*;
use ccontrol_security::{sas, tls, DeviceIdentity};
use std::path::PathBuf;
use tokio::net::TcpStream;

fn usage() -> ! {
    eprintln!("usage: ccontrol-ctl --state DIR (pair ADDR | call ADDR ACTION [ARG])\n  actions: device.info telemetry.get media.status process.list app.list process.terminate <pid:name[:force]> app.close <window_id:pid> pairing.unpair volume.get volume.set <0..1> volume.mute <true|false> media.key <play_pause|next|previous|stop>");
    std::process::exit(2)
}

fn parse_action(name: &str, arg: Option<&String>) -> anyhow::Result<Action> {
    Ok(match name {
        "device.info" => Action::DeviceInfo,
        "telemetry.get" => Action::TelemetryGet,
        "volume.get" => Action::VolumeGet,
        "volume.set" => Action::VolumeSet {
            level: arg.context("level")?.parse()?,
        },
        "volume.mute" => Action::VolumeMute {
            muted: arg.context("true|false")?.parse()?,
        },
        "media.status" => Action::MediaStatus,
        "process.list" => Action::ProcessList,
        "app.list" => Action::AppList,
        "pairing.unpair" => Action::Unpair,
        "process.terminate" => {
            // ARG = pid:name[:force]
            let v: Vec<&str> = arg.context("pid:name[:force]")?.splitn(3, ':').collect();
            Action::ProcessTerminate {
                pid: v[0].parse()?,
                name: v.get(1).context("name")?.to_string(),
                force: v.get(2) == Some(&"force"),
            }
        }
        "app.close" => {
            // ARG = window_id:pid
            let v: Vec<&str> = arg.context("window_id:pid")?.split(':').collect();
            Action::AppClose {
                window_id: v[0].parse()?,
                pid: v.get(1).context("pid")?.parse()?,
            }
        }
        "media.key" => Action::MediaKey {
            key: serde_json::from_str(&format!("\"{}\"", arg.context("key")?))?,
        },
        _ => bail!("unknown action {name}"),
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 || args[1] != "--state" {
        usage()
    }
    let dir = PathBuf::from(&args[2]);
    let cmd = args[3].as_str();
    let addr = args[4].clone();
    let me = DeviceIdentity::load_or_create(&dir, "ccontrol-ctl")?;
    let pin_file = dir.join(format!("pin-{}", addr.replace([':', '.'], "_")));
    let pinned = std::fs::read_to_string(&pin_file).ok();

    let cfg = tls::client_config(&me, pinned.clone())?;
    let tcp = TcpStream::connect(&addr).await?;
    let stream = tokio_rustls::TlsConnector::from(cfg)
        .connect(
            rustls::pki_types::ServerName::try_from("ccontrol.invalid")?,
            tcp,
        )
        .await
        .context("TLS handshake (pinned fingerprint mismatch?)")?;
    let server_fp = tls::peer_fingerprint(stream.get_ref().1.peer_certificates())?;
    let (mut rd, mut wr) = tokio::io::split(stream);
    write_frame(
        &mut wr,
        &Message::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            min_protocol_version: MIN_PROTOCOL_VERSION,
            device_id: me.fingerprint.clone(),
            peer_device_id: server_fp.clone(),
            device_name: "ccontrol-ctl".into(),
            platform: std::env::consts::OS.into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        }),
    )
    .await?;
    let mut ack = match read_frame(&mut rd).await? {
        Message::HelloAck(a) => a,
        Message::Error { error } => bail!("{:?}: {}", error.code, error.message),
        m => bail!("unexpected {m:?}"),
    };

    if !ack.paired {
        if cmd != "pair" {
            bail!("not paired with {addr}; run `pair` first");
        }
        let nc = sas::random_nonce();
        write_frame(
            &mut wr,
            &Message::PairCommit {
                commitment: sas::commitment(&nc),
                requested_scopes: Scope::ALL.to_vec(),
            },
        )
        .await?;
        let ns = match read_frame(&mut rd).await? {
            Message::PairNonce { nonce } => hex::decode(nonce)?,
            m => bail!("unexpected {m:?}"),
        };
        write_frame(
            &mut wr,
            &Message::PairReveal {
                nonce: hex::encode(nc),
            },
        )
        .await?;
        println!(
            "Verification code: {}",
            sas::compute_sas(&server_fp, &me.fingerprint, &nc, &ns)
        );
        println!("Approve on the computer if it shows the same code...");
        loop {
            match read_frame(&mut rd).await? {
                Message::PairAwaitingApproval { .. } => continue,
                Message::PairResult {
                    accepted: true,
                    granted_scopes,
                    ..
                } => {
                    std::fs::write(&pin_file, &server_fp)?;
                    println!("paired; granted {granted_scopes:?}");
                    break;
                }
                Message::PairResult { reason, .. } => bail!("rejected: {reason:?}"),
                Message::Error { error } => bail!("{:?}: {}", error.code, error.message),
                m => bail!("unexpected {m:?}"),
            }
        }
        ack = match read_frame(&mut rd).await? {
            Message::HelloAck(a) => a,
            m => bail!("unexpected {m:?}"),
        };
    }
    println!(
        "connected to {} ({}) scopes={:?}",
        ack.device_name,
        ccontrol_security::short_fingerprint(&ack.device_id),
        ack.granted_scopes
    );

    if cmd == "call" {
        let name = args.get(5).unwrap_or_else(|| usage());
        let action = parse_action(name, args.get(6))?;
        write_frame(&mut wr, &Message::Request { id: 1, action }).await?;
        loop {
            match read_frame(&mut rd).await? {
                Message::Response { result, error, .. } => {
                    if let Some(e) = error {
                        bail!("{:?}: {}", e.code, e.message);
                    }
                    println!("{}", serde_json::to_string_pretty(&result)?);
                    break;
                }
                Message::Event { event } => println!("event: {event:?}"),
                m => bail!("unexpected {m:?}"),
            }
        }
    }
    Ok(())
}
