//! Real-hardware tests (Windows). They play a quiet 440 Hz tone, so they are
//! opt-in:  `cargo test -p ccontrol-audio-engine --test windows_hw -- --ignored --test-threads=1`
#![cfg(windows)]

use ccontrol_audio_engine::device::native;
use ccontrol_audio_engine::tone::Tone;
use ccontrol_protocol::RouteSource;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Render a tone on `endpoint` for `ms` in a background thread.
fn play_tone(endpoint: Option<String>, ms: u64, gain: f32) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let sys = native();
        let mut sink = sys.open_sink(endpoint.as_deref()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            s2.store(true, Ordering::Relaxed);
        });
        let mut tone = Tone::new(440.0, gain);
        sink.run(&mut || tone.next_frame(), &stop).unwrap();
    })
}

fn capture_peak(src: &RouteSource, ms: u64) -> (f32, usize) {
    let sys = native();
    let mut s = sys.open_source(src).unwrap();
    let stop = AtomicBool::new(false);
    let start = Instant::now();
    let (mut peak, mut frames) = (0.0f32, 0);
    while start.elapsed() < Duration::from_millis(ms) {
        let f = s.read_frame(&stop).unwrap().unwrap();
        assert_eq!(f.len(), 960);
        peak = peak.max(ccontrol_audio_engine::peak(&f));
        frames += 1;
    }
    (peak, frames)
}

#[test]
#[ignore]
fn inventory_lists_real_endpoints() {
    let inv = native().inventory().unwrap();
    println!("{inv:#?}");
    assert!(!inv.endpoints.is_empty());
    assert_eq!(inv.endpoints.iter().filter(|e| e.is_default).count(), 1);
}

#[test]
#[ignore]
fn loopback_captures_what_the_default_device_plays() {
    let player = play_tone(None, 1500, 0.1);
    std::thread::sleep(Duration::from_millis(300));
    let (peak, frames) = capture_peak(&RouteSource::Endpoint, 800);
    player.join().unwrap();
    println!("loopback peak {peak:.3} over {frames} frames");
    // ~80 frames in 800 ms: the capture follows real time.
    assert!((60..=100).contains(&frames), "{frames}");
    assert!(
        peak > 0.01,
        "tone not captured (is the default device muted?)"
    );
}

#[test]
#[ignore]
fn loopback_is_silent_paced_when_nothing_plays_on_default() {
    // Play on a *non-default* endpoint: the default's loopback must not hear it.
    let inv = native().inventory().unwrap();
    let Some(other) = inv.endpoints.iter().find(|e| !e.is_default).cloned() else {
        eprintln!("only one output device; skipping");
        return;
    };
    let player = play_tone(Some(other.id), 1500, 0.1);
    std::thread::sleep(Duration::from_millis(300));
    let (peak, frames) = capture_peak(&RouteSource::Endpoint, 800);
    player.join().unwrap();
    println!(
        "default loopback while playing on '{}': peak {peak:.4}, {frames} frames",
        other.name
    );
    assert!(
        (60..=100).contains(&frames),
        "silence must still be paced: {frames}"
    );
    assert!(peak < 0.01);
}

#[test]
#[ignore]
fn session_mute_is_reversible() {
    // Our own render stream creates an audio session owned by this process.
    let player = play_tone(None, 2500, 0.0);
    std::thread::sleep(Duration::from_millis(500));
    let sys = native();
    let pid = std::process::id();
    let before = sys.session_mutes(pid).unwrap();
    assert!(!before.is_empty(), "test process has an audio session");
    for (id, _) in &before {
        assert!(sys.set_session_mute(id, true).unwrap());
    }
    assert!(sys.session_mutes(pid).unwrap().iter().all(|(_, m)| *m));
    for (id, m) in &before {
        sys.set_session_mute(id, *m).unwrap();
    }
    assert_eq!(sys.session_mutes(pid).unwrap(), before);
    assert!(!sys.set_session_mute("no-such-session", true).unwrap());
    player.join().unwrap();
}
