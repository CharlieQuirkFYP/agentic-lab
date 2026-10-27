//! A phone monitor. This startup path never constructs a local model/runtime.
mod network;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result};
use network::{Command, Event as NetworkEvent, Network};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Paragraph, Wrap};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    model_file: String,
    language: String,
    dictionary: String,
    threads: u32,
    max_seconds: u32,
    metrics_enabled: bool,
}
impl Settings {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!((1..=16).contains(&self.threads), "threads must be 1–16");
        anyhow::ensure!(
            (1..=120).contains(&self.max_seconds),
            "maxSeconds must be 1–120"
        );
        anyhow::ensure!(
            self.model_file.len() <= 4096
                && self.language.len() <= 16
                && self.dictionary.len() <= 8000,
            "settings exceed bounds"
        );
        Ok(())
    }
}

pub fn run(bind: SocketAddr, configuration: Option<PathBuf>) -> Result<()> {
    let file_settings: Option<Settings> = configuration
        .map(|path| {
            let metadata = std::fs::metadata(&path)?;
            anyhow::ensure!(metadata.len() <= 16384, "configuration exceeds 16 KiB");
            let settings: Settings = serde_json::from_slice(&std::fs::read(path)?)?;
            settings.validate()?;
            Ok::<_, anyhow::Error>(settings)
        })
        .transpose()?;
    let network = Network::start(bind)?;
    let mut terminal = ratatui::init();
    let result = (|| {
        let mut session = None;
        let mut device = String::new();
        let mut status = "Waiting for the phone to pair".to_owned();
        let mut snapshot = json!({});
        let mut models: Vec<String> = Vec::new();
        let mut draft: Option<Settings> = None;
        let mut dirty = false;
        let mut metrics: Vec<String> = Vec::new();
        let mut remote_drops = 0;
        let mut next_command = 0_u64;
        let mut pending_config: Option<String> = None;
        let mut core_drops: Option<u64> = None;
        loop {
            while let Ok(event) = network.events.try_recv() {
                match event {
                    NetworkEvent::Connected {
                        session: connected,
                        device: name,
                    } => {
                        session = Some(connected);
                        device = name;
                        status = "Paired; awaiting current state (no history replay)".to_owned();
                        draft = None;
                        dirty = false;
                        metrics.clear();
                    }
                    NetworkEvent::Disconnected(reason) => {
                        session = None;
                        pending_config = None;
                        draft = None;
                        dirty = false;
                        status = reason;
                    }
                    NetworkEvent::Message(value) if value["type"] == "ack" => {
                        status = format!(
                            "{}: {}",
                            if value["accepted"] == true {
                                "Applied"
                            } else {
                                "Rejected"
                            },
                            value["message"].as_str().unwrap_or("")
                        );
                        if value["id"].as_str() == pending_config.as_deref() {
                            pending_config = None;
                            if value["accepted"] == true {
                                dirty = false;
                            }
                        }
                    }
                    NetworkEvent::Message(value) => {
                        snapshot = value["snapshot"].clone();
                        models =
                            serde_json::from_value(value["models"].clone()).unwrap_or_default();
                        if !dirty {
                            draft = serde_json::from_value(value["settings"].clone()).ok();
                        }
                        remote_drops = value["dropped_observations"].as_u64().unwrap_or(0);
                        core_drops = value["observer"]["dropped_events"].as_u64();
                        if let Some(raw) = value["batches_json"].as_str() {
                            append_metrics(&mut metrics, raw);
                        }
                    }
                }
            }
            terminal.draw(|frame| {
                let areas = Layout::vertical([Constraint::Length(7), Constraint::Length(12), Constraint::Min(3), Constraint::Length(3)]).split(frame.area());
                frame.render_widget(Paragraph::new(format!(
                    "Listen: ws://{} (use this Mac's LAN address on the phone)\nPairing code: {}\nDevice: {}\n{}\nLoss: core events {}, phone frames {}, desktop frames {}",
                    network.address, network.token, device, status, core_drops.map(|n| n.to_string()).unwrap_or_else(|| "unavailable".to_owned()), remote_drops, network.dropped.load(Ordering::Relaxed)
                )).block(Block::bordered().title("Pheme phone monitor")), areas[0]);
                let settings = draft.as_ref().map(|s| format!(
                    "Selected phone model: {}\nThreads: {}   Max audio: {}s   Metrics: {}\nLanguage: {}   Dictionary: {}\n{}",
                    s.model_file, s.threads, s.max_seconds, s.metrics_enabled,
                    if s.language.is_empty() { "automatic" } else { &s.language }, s.dictionary,
                    if dirty { "Draft changed; Enter applies" } else { "Phone settings" }
                )).unwrap_or_else(|| "Waiting for settings".to_owned());
                frame.render_widget(Paragraph::new(format!("State: {}   Active run: {}\nApplied threads: {}   Metrics: {}\n{}",
                    snapshot["state"].as_str().unwrap_or("unavailable"),
                    snapshot["active_run"].as_str().unwrap_or("none"),
                    snapshot["configuration"]["threads"], snapshot["configuration"]["metrics_enabled"], settings
                )).wrap(Wrap { trim: false }).block(Block::bordered().title("Configuration")), areas[1]);
                frame.render_widget(Paragraph::new(metrics.join("\n")).wrap(Wrap { trim: false })
                    .block(Block::bordered().title("Core metrics (unavailable values stay unavailable)")), areas[2]);
                frame.render_widget(Paragraph::new("m model · -/+ threads · e metrics · f load --config draft · Enter apply · r inspect · q quit")
                    .wrap(Wrap { trim: false }).block(Block::bordered()), areas[3]);
            })?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('q') {
                break;
            }
            let Some(active_session) = session else {
                continue;
            };
            match key.code {
                KeyCode::Char('m') => {
                    if let Some(settings) = draft.as_mut() {
                        if !models.is_empty() {
                            let index = models
                                .iter()
                                .position(|m| *m == settings.model_file)
                                .map(|i| (i + 1) % models.len())
                                .unwrap_or(0);
                            settings.model_file.clone_from(&models[index]);
                            dirty = true;
                        }
                    }
                }
                KeyCode::Char('-') => {
                    if let Some(s) = draft.as_mut() {
                        s.threads = s.threads.saturating_sub(1).max(1);
                        dirty = true;
                    }
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    if let Some(s) = draft.as_mut() {
                        s.threads = (s.threads + 1).min(16);
                        dirty = true;
                    }
                }
                KeyCode::Char('e') => {
                    if let Some(s) = draft.as_mut() {
                        s.metrics_enabled = !s.metrics_enabled;
                        dirty = true;
                    }
                }
                KeyCode::Char('f') => {
                    if let Some(settings) = &file_settings {
                        draft = Some(settings.clone());
                        dirty = true;
                    }
                }
                KeyCode::Enter | KeyCode::Char('r') => {
                    next_command += 1;
                    let id = format!("{active_session}-{next_command}");
                    let value = if key.code == KeyCode::Enter {
                        if pending_config.is_some() {
                            status =
                                "Waiting for the previous configuration acknowledgement".to_owned();
                            continue;
                        }
                        let Some(settings) = &draft else { continue };
                        if let Err(error) = settings.validate() {
                            status = error.to_string();
                            continue;
                        }
                        json!({"version":1,"type":"configure","id":id,"settings":settings})
                    } else {
                        json!({"version":1,"type":"inspect","id":id})
                    };
                    status = match network.commands.try_send(Command {
                        session: active_session,
                        value,
                    }) {
                        Ok(()) => {
                            if key.code == KeyCode::Enter {
                                pending_config = Some(id);
                            }
                            "Command sent; waiting for acknowledgement".to_owned()
                        }
                        Err(_) => "Command queue unavailable; refresh the connection".to_owned(),
                    };
                }
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    })();
    ratatui::restore();
    drop(network);
    result.context("phone monitor failed")
}

fn append_metrics(lines: &mut Vec<String>, raw: &str) {
    let Ok(batches) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    for batch in batches.as_array().into_iter().flatten() {
        for event in batch["events"].as_array().into_iter().flatten() {
            let value = if event["value"].is_null() {
                format!(
                    "unavailable ({})",
                    event["unavailable_reason"]
                        .as_str()
                        .unwrap_or("not measured")
                )
            } else {
                event["value"].to_string()
            };
            lines.push(format!(
                "{}: {} {}",
                event["name"].as_str().unwrap_or("?"),
                value,
                event["unit"].as_str().unwrap_or("")
            ));
        }
    }
    if lines.len() > 32 {
        lines.drain(..lines.len() - 32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_is_bounded_and_preserves_unavailable_measurements() {
        let mut lines = Vec::new();
        for _ in 0..40 {
            append_metrics(
                &mut lines,
                r#"[{"events":[{"name":"power","value":null,"unit":"watts","unavailable_reason":"no sensor"}]}]"#,
            );
        }
        assert_eq!(lines.len(), 32);
        assert!(lines[0].contains("unavailable (no sensor)"));
    }
}
