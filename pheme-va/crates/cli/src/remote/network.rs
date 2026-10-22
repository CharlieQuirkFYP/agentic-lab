use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{protocol::WebSocketConfig, Message, WebSocket};

pub(super) const MAX_MESSAGE_BYTES: usize = 256 * 1024;

pub(super) struct Command {
    pub session: u64,
    pub value: Value,
}
pub(super) enum Event {
    Connected { session: u64, device: String },
    Disconnected(String),
    Message(Value),
}

pub(super) struct Network {
    pub address: SocketAddr,
    pub token: String,
    pub commands: mpsc::SyncSender<Command>,
    pub events: mpsc::Receiver<Event>,
    pub dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Network {
    pub fn start(address: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(address).context("could not bind monitor")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("pairing randomness: {error}"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let (command_tx, command_rx) = mpsc::sync_channel::<Command>(8);
        let (event_tx, event_rx) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_dropped = Arc::clone(&dropped);
        let worker_token = token.clone();
        let worker = std::thread::spawn(move || {
            let mut session = 0;
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        session += 1;
                        let outcome = serve(
                            stream,
                            &worker_token,
                            session,
                            &command_rx,
                            &event_tx,
                            &worker_stop,
                            &worker_dropped,
                        );
                        // No commands survive a lost connection.
                        while command_rx.try_recv().is_ok() {}
                        let reason = outcome
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "Phone disconnected".to_owned());
                        // Lifecycle messages must not be silently lost behind telemetry.
                        send_lifecycle(&event_tx, Event::Disconnected(reason), &worker_stop);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            token,
            commands: command_tx,
            events: event_rx,
            dropped,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn send_lifecycle(tx: &mpsc::SyncSender<Event>, mut event: Event, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match tx.try_send(event) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => return,
            Err(mpsc::TrySendError::Full(returned)) => {
                event = returned;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[derive(Deserialize)]
struct Hello {
    version: u32,
    #[serde(rename = "type")]
    kind: String,
    token: String,
    device: String,
}

fn authenticate(text: &str, token: &str) -> Result<String> {
    let hello: Hello = serde_json::from_str(text).context("invalid pairing message")?;
    if hello.version != 1
        || hello.kind != "hello"
        || hello.token != token
        || hello.device.len() > 128
    {
        bail!("pairing rejected");
    }
    Ok(hello.device)
}

fn serve(
    stream: TcpStream,
    token: &str,
    session: u64,
    commands: &mpsc::Receiver<Command>,
    events: &mpsc::SyncSender<Event>,
    stop: &AtomicBool,
    dropped: &AtomicU64,
) -> Result<()> {
    // Darwin inherits the listener's nonblocking flag on accepted sockets.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut socket = tungstenite::accept_with_config(stream, Some(config))
        .context("WebSocket handshake failed")?;
    let Message::Text(hello) = socket.read()? else {
        bail!("expected pairing message");
    };
    let device = authenticate(&hello, token)?;
    socket.send(Message::text(
        json!({"version":1,"type":"paired"}).to_string(),
    ))?;
    send_lifecycle(events, Event::Connected { session, device }, stop);
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))?;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(command) = commands.try_recv() {
            if command.session == session {
                socket.send(Message::text(command.value.to_string()))?;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let value = parse_event(&text)?;
                if value["type"] == "ack" {
                    send_lifecycle(events, Event::Message(value), stop);
                } else if events.try_send(Event::Message(value)).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => {
                socket.flush()?;
            }
            Ok(Message::Pong(_)) => {}
            Ok(_) => bail!("only JSON text messages are supported"),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(error) => return Err(error.into()),
        }
    }
    close(&mut socket);
    Ok(())
}

fn close(socket: &mut WebSocket<TcpStream>) {
    let _ = socket.close(None);
}

pub(super) fn parse_event(text: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(text).context("invalid phone JSON")?;
    if value["version"] != 1 || !matches!(value["type"].as_str(), Some("observation" | "ack")) {
        bail!("unsupported phone event");
    }
    if value["type"] == "observation" {
        if !value["snapshot"].is_object()
            || !value["models"].is_array()
            || !value["settings"].is_object()
            || !value["batches_json"].is_string()
        {
            bail!("incomplete phone observation");
        }
        let batches: Value = serde_json::from_str(value["batches_json"].as_str().unwrap())?;
        if !batches.is_array() {
            bail!("metric batches must be an array");
        }
    } else if !value["id"].is_string() || !value["accepted"].is_boolean() {
        bail!("incomplete acknowledgement");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_and_event_validation_reject_bad_input() {
        assert!(authenticate(
            r#"{"version":1,"type":"hello","token":"wrong","device":"phone"}"#,
            "expected"
        )
        .is_err());
        assert!(parse_event("not JSON").is_err());
        assert!(parse_event(r#"{"version":9,"type":"ack"}"#).is_err());
        assert!(parse_event(r#"{"version":1,"type":"observation","snapshot":{},"settings":{},"models":[],"batches_json":"{}"}"#).is_err());
        assert!(parse_event(
            r#"{"version":1,"type":"ack","id":"1","accepted":false,"message":"busy"}"#
        )
        .is_ok());
    }

    #[test]
    fn fake_phone_pairs_disconnects_and_stale_commands_do_not_replay() {
        let network = Network::start("127.0.0.1:0".parse().unwrap()).unwrap();
        let connect = || {
            let stream = TcpStream::connect(network.address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let (mut socket, _) =
                tungstenite::client(format!("ws://{}", network.address), stream).unwrap();
            socket
                .send(Message::text(
                    json!({"version":1,"type":"hello","token":network.token,"device":"fake phone"})
                        .to_string(),
                ))
                .unwrap();
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            socket
        };
        let mut first = connect();
        let Event::Connected { session, .. } =
            network.events.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("expected connect")
        };
        first.close(None).unwrap();
        drop(first);
        assert!(matches!(
            network.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Disconnected(_)
        ));
        let mut second = connect();
        let Event::Connected {
            session: second_session,
            ..
        } = network.events.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("expected reconnect")
        };
        assert_ne!(session, second_session);
        network
            .commands
            .send(Command {
                session,
                value: json!({"type":"configure","id":"stale"}),
            })
            .unwrap();
        network
            .commands
            .send(Command {
                session: second_session,
                value: json!({"version":1,"type":"inspect","id":"fresh"}),
            })
            .unwrap();
        let Message::Text(message) = second.read().unwrap() else {
            panic!("expected fresh command")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&message).unwrap()["id"],
            "fresh"
        );
        second
            .send(Message::text(
                r#"{"version":1,"type":"ack","id":"fresh","accepted":false,"message":"busy"}"#,
            ))
            .unwrap();
        assert!(matches!(
            network.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Message(_)
        ));
    }
}
