//! Developer controller CLI: exercises the same protocol as the Android app.
//!
//! ```text
//! ccontrol-ctl --state ./ctl pair 192.168.1.20:47800          # prints SAS, waits for approval
//! ccontrol-ctl --state ./ctl call 192.168.1.20:47800 volume.get
//! ccontrol-ctl --state ./ctl call 192.168.1.20:47800 volume.set 0.4
//! ccontrol-ctl --state ./ctl route SRC_ADDR DST_ADDR tone|output|pid:N SECS [DST_ENDPOINT_ID] [--silence]
//! ```

use anyhow::{bail, Context};
use ccontrol_protocol::*;
use ccontrol_security::{sas, tls, DeviceIdentity};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

fn usage() -> ! {
    eprintln!(
        "usage: ccontrol-ctl --state DIR (pair ADDR | call ADDR ACTION [ARG] | route SRC DST tone|output|pid:N SECS [ENDPOINT_ID] [--silence])\n  actions: device.info telemetry.get media.status process.list app.list process.terminate <pid:name[:force]> app.close <window_id:pid> pairing.unpair audio.devices route.list volume.get volume.set <0..1> volume.mute <true|false> media.key <play_pause|next|previous|stop>"
    );
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
        "audio.devices" => Action::AudioDevices,
        "route.list" => Action::RouteList,
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

struct Session {
    rd: ReadHalf<TlsStream<TcpStream>>,
    wr: WriteHalf<TlsStream<TcpStream>>,
    ack: HelloAck,
    next_id: u64,
}

impl Session {
    async fn connect(
        dir: &Path,
        me: &DeviceIdentity,
        addr: &str,
        allow_pair: bool,
    ) -> anyhow::Result<Self> {
        let pin_file = dir.join(format!("pin-{}", addr.replace([':', '.'], "_")));
        let pinned = std::fs::read_to_string(&pin_file).ok();
        let cfg = tls::client_config(me, pinned)?;
        let tcp = TcpStream::connect(addr)
            .await
            .with_context(|| format!("connecting to {addr}"))?;
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
            if !allow_pair {
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
        Ok(Self {
            rd,
            wr,
            ack,
            next_id: 0,
        })
    }

    async fn request(&mut self, action: Action) -> anyhow::Result<ActionResult> {
        self.next_id += 1;
        let id = self.next_id;
        write_frame(&mut self.wr, &Message::Request { id, action }).await?;
        loop {
            match read_frame(&mut self.rd).await? {
                Message::Response {
                    id: rid,
                    result,
                    error,
                } if rid == id => {
                    if let Some(e) = error {
                        bail!("{:?}: {}", e.code, e.message);
                    }
                    return result.context("empty response");
                }
                Message::Event { event } => eprintln!("event: {event:?}"),
                Message::Pong { .. } => {}
                m => bail!("unexpected {m:?}"),
            }
        }
    }
}

fn host_of(addr: &str) -> String {
    addr.rsplit_once(':')
        .map(|(h, _)| h.to_owned())
        .unwrap_or_else(|| addr.to_owned())
}

async fn route(dir: &Path, me: &DeviceIdentity, args: &[String]) -> anyhow::Result<()> {
    let (src_addr, dst_addr) = (&args[0], &args[1]);
    let source = match args.get(2).map(String::as_str) {
        Some("tone") => RouteSource::TestTone,
        Some("output") => RouteSource::Endpoint,
        Some(s) if s.starts_with("pid:") => RouteSource::Session {
            pid: s[4..].parse()?,
            app_name: None,
        },
        _ => usage(),
    };
    let secs: u64 = args.get(3).context("SECS")?.parse()?;
    let endpoint = args.get(4).filter(|a| !a.starts_with("--")).cloned();
    let silence = args.iter().any(|a| a == "--silence");
    let lease = 15u32;

    let mut src = Session::connect(dir, me, src_addr, false).await?;
    let mut dst = Session::connect(dir, me, dst_addr, false).await?;
    let route_id = format!("ctl-{}", std::process::id());
    let key = ccontrol_audio_engine::packet::random_key_hex();

    let t0 = std::time::Instant::now();
    let sink = match dst
        .request(Action::SinkPrepare {
            route_id: route_id.clone(),
            key: key.clone(),
            endpoint_id: endpoint,
            lease_secs: lease,
        })
        .await?
    {
        ActionResult::Sink(s) => s,
        other => bail!("{other:?}"),
    };
    println!(
        "sink ready on {} (udp {}) in {:?}",
        sink.endpoint_name,
        sink.udp_port,
        t0.elapsed()
    );
    let r = match src
        .request(Action::RouteStart {
            route_id: route_id.clone(),
            source,
            dest_host: host_of(dst_addr),
            dest_port: sink.udp_port,
            key,
            lease_secs: lease,
            silence_local: silence,
        })
        .await?
    {
        ActionResult::Route(r) => r,
        other => bail!("{other:?}"),
    };
    println!(
        "route {:?} after {:?}; capture={:?} local_silenced={} note={:?} error={:?}",
        r.state,
        t0.elapsed(),
        r.capture,
        r.local_silenced,
        r.note,
        r.error
    );
    if r.state != RouteState::Active {
        let _ = dst
            .request(Action::SinkStop {
                route_id: route_id.clone(),
            })
            .await;
        bail!("route did not become active");
    }
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_secs(1)).await;
        src.request(Action::LeaseRenew {
            route_id: route_id.clone(),
        })
        .await?;
        dst.request(Action::LeaseRenew {
            route_id: route_id.clone(),
        })
        .await?;
        let (routes, _) = match src.request(Action::RouteList).await? {
            ActionResult::Routes { routes, sinks } => (routes, sinks),
            _ => continue,
        };
        let (_, sinks) = match dst.request(Action::RouteList).await? {
            ActionResult::Routes { routes, sinks } => (routes, sinks),
            _ => continue,
        };
        let r = routes
            .iter()
            .find(|r| r.route_id == route_id)
            .context("route gone")?;
        let s = sinks
            .iter()
            .find(|s| s.route_id == route_id)
            .context("sink gone")?;
        println!(
            "src {:?} sent={} {:.0}kb/s cap_peak={:.3} rtt={:?}ms latency~{:?}ms | dst {:?} played={} lost={} late={} jitter={:.2}ms buf={:.0}ms out_peak={:.3}",
            r.state, r.stats.packets, r.stats.bitrate_kbps, r.stats.peak,
            r.stats.rtt_ms.map(|v| (v * 100.0).round() / 100.0), r.stats.latency_ms.map(|v| v.round()),
            s.state, s.stats.frames_played, s.stats.lost, s.stats.late, s.stats.jitter_ms, s.stats.buffer_ms, s.stats.peak
        );
        if r.state != RouteState::Active {
            println!(
                "route ended: outcome={:?} error={:?} note={:?}",
                r.outcome, r.error, r.note
            );
            break;
        }
    }
    if let ActionResult::Route(r) = src
        .request(Action::RouteStop {
            route_id: route_id.clone(),
        })
        .await?
    {
        println!("stopped: state={:?} outcome={:?}", r.state, r.outcome);
    }
    if let ActionResult::Sink(s) = dst.request(Action::SinkStop { route_id }).await? {
        println!(
            "sink: state={:?} frames_played={} lost={}",
            s.state, s.stats.frames_played, s.stats.lost
        );
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 || args[1] != "--state" {
        usage()
    }
    let dir = PathBuf::from(&args[2]);
    let me = DeviceIdentity::load_or_create(&dir, "ccontrol-ctl")?;
    match args[3].as_str() {
        "pair" => {
            let s = Session::connect(&dir, &me, &args[4], true).await?;
            println!(
                "connected to {} ({}) scopes={:?}",
                s.ack.device_name,
                ccontrol_security::short_fingerprint(&s.ack.device_id),
                s.ack.granted_scopes
            );
        }
        "call" => {
            let mut s = Session::connect(&dir, &me, &args[4], false).await?;
            let name = args.get(5).unwrap_or_else(|| usage());
            let result = s.request(parse_action(name, args.get(6))?).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        "route" if args.len() >= 8 => route(&dir, &me, &args[4..]).await?,
        _ => usage(),
    }
    Ok(())
}
