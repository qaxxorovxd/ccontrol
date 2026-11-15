//! Parser for `pw-dump` JSON (PipeWire object graph). Portable so it is
//! unit-tested on every platform.

use ccontrol_protocol::{AudioEndpoint, AudioSession};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct PwGraph {
    pub sinks: Vec<AudioEndpoint>,
    /// Application output streams.
    pub streams: Vec<(u32, AudioSession)>,
}

fn prop<'a>(o: &'a Value, k: &str) -> Option<&'a Value> {
    o.get("info")?.get("props")?.get(k)
}

fn prop_str(o: &Value, k: &str) -> Option<String> {
    prop(o, k).and_then(|v| v.as_str()).map(str::to_owned)
}

fn prop_u32(o: &Value, k: &str) -> Option<u32> {
    let v = prop(o, k)?;
    v.as_u64()
        .map(|n| n as u32)
        .or_else(|| v.as_str()?.parse().ok())
}

pub fn parse(json: &str) -> Result<PwGraph, String> {
    let all: Vec<Value> = serde_json::from_str(json).map_err(|e| format!("pw-dump output: {e}"))?;
    // Default sink name lives in the "default" metadata object.
    let default_sink = all
        .iter()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Metadata"))
        .filter_map(|o| o.get("metadata")?.as_array())
        .flatten()
        .find(|m| m.get("key").and_then(Value::as_str) == Some("default.audio.sink"))
        .and_then(|m| m.get("value")?.get("name")?.as_str().map(str::to_owned));

    let mut sinks = vec![];
    let mut streams = vec![];
    for o in all
        .iter()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node"))
    {
        let id = o.get("id").and_then(Value::as_u64).unwrap_or(0) as u32;
        match prop_str(o, "media.class").as_deref() {
            Some("Audio/Sink") => {
                let name = prop_str(o, "node.name").unwrap_or_default();
                sinks.push(AudioEndpoint {
                    is_default: Some(&name) == default_sink.as_ref(),
                    name: prop_str(o, "node.description").unwrap_or_else(|| name.clone()),
                    id: name,
                });
            }
            Some("Stream/Output/Audio") => {
                let state = o
                    .get("info")
                    .and_then(|i| i.get("state"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                streams.push((
                    id,
                    AudioSession {
                        session_id: id.to_string(),
                        pid: prop_u32(o, "application.process.id").unwrap_or(0),
                        app_name: prop_str(o, "application.name")
                            .or_else(|| prop_str(o, "application.process.binary"))
                            .unwrap_or_else(|| format!("node {id}")),
                        display_name: prop_str(o, "media.name"),
                        active: state == "running",
                        peak: 0.0,
                        muted: false,
                        volume: 1.0,
                        system_sounds: false,
                        process_capture: true,
                        process_capture_reason: None,
                        can_silence_local: false,
                    },
                ));
            }
            _ => {}
        }
    }
    Ok(PwGraph { sinks, streams })
}

#[cfg(test)]
mod tests {
    const SAMPLE: &str = r#"[
      {"id":0,"type":"PipeWire:Interface:Core","info":{}},
      {"id":45,"type":"PipeWire:Interface:Node","info":{"state":"suspended","props":{"media.class":"Audio/Sink","node.name":"alsa_output.pci-0000_00_1f.3.analog-stereo","node.description":"Built-in Audio Analog Stereo"}}},
      {"id":52,"type":"PipeWire:Interface:Node","info":{"state":"idle","props":{"media.class":"Audio/Sink","node.name":"bluez_output.AA_BB","node.description":"Headphones"}}},
      {"id":88,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Stream/Output/Audio","application.name":"Spotify","application.process.id":"4242","media.name":"Spotify"}}},
      {"id":91,"type":"PipeWire:Interface:Node","info":{"state":"idle","props":{"media.class":"Stream/Output/Audio","application.process.binary":"firefox","application.process.id":777}}},
      {"id":30,"type":"PipeWire:Interface:Metadata","metadata":[{"subject":0,"key":"default.audio.sink","type":"Spa:String:JSON","value":{"name":"bluez_output.AA_BB"}}]}
    ]"#;

    #[test]
    fn parses_sinks_streams_and_default() {
        let g = super::parse(SAMPLE).unwrap();
        assert_eq!(g.sinks.len(), 2);
        assert!(
            g.sinks
                .iter()
                .find(|s| s.name == "Headphones")
                .unwrap()
                .is_default
        );
        assert_eq!(g.streams.len(), 2);
        let (id, spotify) = &g.streams[0];
        assert_eq!(*id, 88);
        assert_eq!(spotify.pid, 4242);
        assert!(spotify.active);
        assert_eq!(g.streams[1].1.app_name, "firefox");
        assert_eq!(g.streams[1].1.pid, 777);
        assert!(super::parse("not json").is_err());
    }
}
