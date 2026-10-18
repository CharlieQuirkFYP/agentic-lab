use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

#[derive(Clone)]
struct Request {
    path: String,
    body: Vec<u8>,
    content_type: String,
}

struct FakeGo {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    stream_closed: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeGo {
    fn start(hold_reply: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let stream_closed = Arc::new(AtomicBool::new(false));
        let collected = Arc::clone(&requests);
        let stopping = Arc::clone(&stop);
        let closed = Arc::clone(&stream_closed);
        let thread = std::thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let requests = Arc::clone(&collected);
                        let stop = Arc::clone(&stopping);
                        let closed = Arc::clone(&closed);
                        clients.push(std::thread::spawn(move || {
                            serve(socket, requests, stop, closed, hold_reply)
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fake listener failed: {error}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            stream_closed,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeGo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn serve(
    mut socket: TcpStream,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    hold_reply: bool,
) {
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 4096];
        let Ok(length) = socket.read(&mut buffer) else {
            return;
        };
        if length == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..length]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(bytes.len() < MAX_JSON_BYTES);
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let path = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let content_type = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.trim().to_owned())
        })
        .unwrap_or_default();
    while bytes.len() - header_end < length {
        let mut buffer = [0_u8; 4096];
        let count = socket.read(&mut buffer).unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = bytes[header_end..header_end + length].to_vec();
    requests.lock().unwrap().push(Request {
        path: path.clone(),
        body,
        content_type,
    });
    let (status, content_type, body) = match path.as_str() {
        "/api/v1/voice/inspect" => (200, "application/json", serde_json::json!({
            "history":[{"role":"user","content":"web only"}],
            "current_turn":{"turn_id":"web-turn","status":"awaiting_review","transcript":"web original","approved_text":null,"reply":"web partial","error":null,"timings":{}},
            "stt":{"name":"server-stt","ready":true},"reply":{"name":"server-reply","ready":true},"role":null,"busy":false
        }).to_string()),
        "/api/v1/voice/turns/web-turn/submit" => (200,"application/json","{\"status\":\"generating\"}".into()),
        "/api/v1/voice/turns/stale/submit" => (409,"application/json","{\"error\":{\"code\":\"stale_turn\",\"message\":\"no longer reviewable\"}}".into()),
        "/api/v1/voice/transcribe" => (200,"application/json",serde_json::json!({
            "status":"Speech","raw_text":"raw test","text":"test transcript","model_id":"server-stt","stt_backend":"fake","processing_time_ms":7
        }).to_string()),
        "/api/v1/voice/test/reply" if hold_reply => {
            if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: reply.started\ndata: {}\n\n").is_err() { return; }
            while !stop.load(Ordering::Relaxed) {
                if socket.write_all(b": heartbeat\n\n").is_err() { closed.store(true,Ordering::Relaxed); break; }
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        "/api/v1/voice/test/reply" => (200,"text/event-stream","event: reply.started\ndata: {}\n\nevent: reply.delta\ndata: {\"text\":\"界🙂\"}\n\nevent: reply.completed\ndata: {\"text\":\"authoritative final\"}\n\n".into()),
        _ => (404,"application/json","{}".into()),
    };
    let response = format!("HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
    let _ = socket.write_all(response.as_bytes());
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "fake operation timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn go_transport_separates_inspection_submit_reply_and_stateless_audio() {
    let server = FakeGo::start(false);
    let mut connection = Connection::start(&server.url).unwrap();
    let mut snapshot = None;
    wait_for(|| {
        if let Some(result) = connection.inspection() {
            snapshot = Some(result.unwrap());
        }
        snapshot.is_some()
    });
    assert_eq!(snapshot.as_ref().unwrap().history[0].content, "web only");
    connection
        .send(Command::Submit {
            turn_id: "web-turn".into(),
            text: "edited 界 question".into(),
        })
        .unwrap();
    let mut accepted = false;
    wait_for(|| {
        while let Some(event) = connection.event() {
            if let Event::Submitted {
                turn_id,
                text,
                result,
            } = event
            {
                assert_eq!(turn_id, "web-turn");
                assert_eq!(text, "edited 界 question");
                result.unwrap();
                accepted = true;
            }
        }
        accepted
    });
    connection
        .send(Command::Reply {
            request: 1,
            text: "isolated test".into(),
        })
        .unwrap();
    let mut delta = String::new();
    let mut completed = false;
    wait_for(|| {
        while let Some(event) = connection.event() {
            match event {
                Event::ReplyDelta { request: 1, text } => delta.push_str(&text),
                Event::ReplyCompleted { request: 1, text } => {
                    assert_eq!(text, "authoritative final");
                    completed = true;
                }
                Event::TestFailed { error, .. } => panic!("{error}"),
                _ => {}
            }
        }
        completed
    });
    assert_eq!(delta, "界🙂");
    let audio = AudioBuffer::new(16_000, 1, vec![0.1; 160]).unwrap();
    connection
        .send(Command::Transcribe {
            request: 2,
            audio: AudioInput::Microphone(audio),
            max_seconds: 1,
        })
        .unwrap();
    let mut transcribed = false;
    wait_for(|| {
        while let Some(event) = connection.event() {
            if let Event::Transcript { request: 2, result } = event {
                assert_eq!(result.unwrap().text, "test transcript");
                transcribed = true;
            }
        }
        transcribed
    });
    drop(connection);
    let requests = server.requests.lock().unwrap();
    let submit = requests
        .iter()
        .find(|request| request.path.ends_with("/submit"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&submit.body).unwrap(),
        serde_json::json!({"text":"edited 界 question"})
    );
    let reply = requests
        .iter()
        .find(|request| request.path.ends_with("/test/reply"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&reply.body).unwrap(),
        serde_json::json!({"text":"isolated test"})
    );
    let audio = requests
        .iter()
        .find(|request| request.path.ends_with("/transcribe"))
        .unwrap();
    assert_eq!(audio.content_type, "audio/wav");
    assert_eq!(
        AudioBuffer::from_wav(&audio.body).unwrap().samples.len(),
        160
    );
    assert!(requests
        .iter()
        .all(|request| !request.path.contains("model")
            && !request.path.ends_with("/cancel")
            && !request.path.ends_with("/reset")));
}

#[test]
fn dropping_client_closes_only_the_test_stream_and_never_cancels_web_work() {
    let server = FakeGo::start(true);
    let mut connection = Connection::start(&server.url).unwrap();
    connection
        .send(Command::Reply {
            request: 1,
            text: "test".into(),
        })
        .unwrap();
    wait_for(|| matches!(connection.event(), Some(Event::ReplyStarted { request: 1 })));
    let started = Instant::now();
    drop(connection);
    assert!(started.elapsed() < Duration::from_secs(1));
    wait_for(|| server.stream_closed.load(Ordering::Relaxed));
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.path.ends_with("/inspect")
                || request.path.ends_with("/test/reply"))
    );
}

#[test]
fn submission_error_is_correlated_and_not_automatically_retried() {
    let server = FakeGo::start(false);
    let mut connection = Connection::start(&server.url).unwrap();
    connection
        .send(Command::Submit {
            turn_id: "stale".into(),
            text: "retain me".into(),
        })
        .unwrap();
    let mut failed = false;
    wait_for(|| {
        if let Some(Event::Submitted {
            turn_id,
            text,
            result,
        }) = connection.event()
        {
            assert_eq!(turn_id, "stale");
            assert_eq!(text, "retain me");
            assert!(result.unwrap_err().contains("stale_turn"));
            failed = true;
        }
        failed
    });
    drop(connection);
    assert_eq!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path.ends_with("/submit"))
            .count(),
        1
    );
}
