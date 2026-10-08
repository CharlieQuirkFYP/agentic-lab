//! Local Rust runtime bridge. No HTTP or SSE in Chat.
use super::connected::{prepare_wav, AudioInput};
use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};
use va_core::chat::ChatSnapshot;
use va_runtime::{conversations, events::EventKind, AgentRuntime, RequestOptions, TurnInput};
pub enum Command {
    New,
    Test(super::connected::Command),
    Audio {
        id: String,
        audio: AudioInput,
        source: String,
        max_seconds: u32,
        key: String,
    },
    Text {
        id: String,
        text: String,
        key: String,
    },
    Submit {
        id: String,
        turn: String,
        text: String,
    },
    Cancel {
        id: String,
        turn: String,
    },
    Finish {
        id: String,
    },
}

pub enum Event {
    Created(ChatSnapshot),
    Snapshot(ChatSnapshot),
    Metrics {
        id: String,
        events: Vec<metrics::MetricEvent>,
        truncated: bool,
    },
    Test(super::connected::Event),
    Run {
        id: String,
        run: Box<va_core::chat::ChatRun>,
    },
    Submitted,
    Error {
        action: &'static str,
        message: String,
    },
}

pub struct Connection {
    sender: mpsc::Sender<Command>,
    receiver: mpsc::Receiver<Event>,
    stop: watch::Sender<bool>,
    join: Option<std::thread::JoinHandle<()>>,
}
async fn publish(
    agent: &AgentRuntime,
    id: &str,
    events: &mpsc::Sender<Event>,
    cursor: &mut u64,
) -> Result<()> {
    let snapshot = conversations::snapshot(agent, id, false)?;
    let ids = snapshot
        .runs
        .iter()
        .map(|r| r.run_id.clone())
        .collect::<Vec<_>>();
    events.send(Event::Snapshot(snapshot)).await?;
    loop {
        let page = conversations::metrics(agent, id, *cursor)?;
        let full = page.events.len() == 256;
        if page.truncated {
            for run in &ids {
                events
                    .send(Event::Run {
                        id: id.into(),
                        run: Box::new(conversations::run(agent, id, run)?),
                    })
                    .await?;
            }
        }
        *cursor = page.next_cursor;
        if !page.events.is_empty() || page.truncated {
            events
                .send(Event::Metrics {
                    id: id.into(),
                    events: page.events,
                    truncated: page.truncated,
                })
                .await?;
        }
        if !full {
            break;
        }
    }
    Ok(())
}
impl Connection {
    pub fn start(agent: AgentRuntime) -> Result<Self> {
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (sender, mut commands) = mpsc::channel(8);
        let (events, receiver) = mpsc::channel(256);
        let (stop, mut stopping) = watch::channel(false);
        let join = std::thread::Builder::new().name("pheme-local-chat".into()).spawn(move || executor.block_on(async move {
            let mut changes = agent.voice.subscribe();
            let mut selected:Option<String> = None;
            let mut cursor = 0;
            let mut test_cancel:Option<std::sync::Arc<std::sync::atomic::AtomicBool>> = None;
            let mut tests = Vec::new();
            loop {
                tokio::select! {
                    _ = stopping.changed() => break,
                    command = commands.recv() => {
                        let Some(command) = command else { break; };
                        let action = match &command { Command::New=>"new", Command::Audio{..}|Command::Text{..}=>"start",Command::Submit{..}=>"submit",Command::Cancel{..}=>"cancel",Command::Finish{..}=>"finish",Command::Test(_)=>"test" };
                        let result:Result<()> = async {
                            match command {
                                Command::New => {
                                    let snapshot = conversations::new_conversation(&agent)?;
                                    selected = Some(snapshot.conversation_id.clone()); cursor=0;
                                    events.send(Event::Created(snapshot)).await?;
                                }
                                Command::Audio {id,audio,source,max_seconds,key} => {
                                    let input = TurnInput::Audio(prepare_wav(audio,max_seconds)?);
                                    let _stream = conversations::start_turn(&agent,&id,input,RequestOptions { idempotency_key:Some(key),audio_source:source,..Default::default() },false).await?;
                                }
                                Command::Text {id,text,key} => {
                                    let _stream = conversations::start_turn(&agent,&id,TurnInput::Text(text),RequestOptions {idempotency_key:Some(key),..Default::default()},true).await?;
                                    events.send(Event::Submitted).await?;
                                }
                                Command::Submit {id,turn,text} => { conversations::approve(&agent,&id,&turn,text)?; events.send(Event::Submitted).await?; }
                                Command::Cancel {id,turn} => { conversations::stop_turn(&agent,&id,&turn)?; }
                                Command::Finish {id} => { let snapshot = conversations::finish(&agent,&id)?; events.send(Event::Snapshot(snapshot)).await?; }
                                Command::Test(command) => {
                                    use super::connected::{Command as T,Event as E};
                                    match command {
                                        T::CancelTest => { if let Some(flag) = &test_cancel { flag.store(true,std::sync::atomic::Ordering::Release); } }
                                        T::Reply {request,text} => {
                                            let mut stream = va_runtime::voice::test_reply(agent.clone(),text,RequestOptions::default())?;
                                            test_cancel = stream.cancel_on_drop.clone();
                                            let events = events.clone();
                                            tests.push(tokio::spawn(async move {
                                                while let Some(event) = stream.receiver.recv().await {
                                                    let event = match event.kind {
                                                        EventKind::ReplyStarted => E::ReplyStarted {request},
                                                        EventKind::ReplyDelta {text} => E::ReplyDelta {request,text},
                                                        EventKind::ReplyCompleted {text} => E::ReplyCompleted {request,text},
                                                        EventKind::TurnFailed {error} => E::TestFailed {request,error:error.map(|e|e.message.into()).unwrap_or_else(||"Test failed.".into())},
                                                        EventKind::TurnCancelled => E::TestFailed {request,error:"Test cancelled.".into()},
                                                        _ => continue,
                                                    };
                                                    if events.send(Event::Test(event)).await.is_err() { if let Some(flag) = &stream.cancel_on_drop { flag.store(true,std::sync::atomic::Ordering::Release); } break; }
                                                }
                                            }));
                                        }
                                        T::Transcribe {request,audio,max_seconds} => {
                                            let wav = prepare_wav(audio,max_seconds)?;
                                            let audio = va_core::AudioBuffer::from_wav(&wav)?;
                                            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)); test_cancel = Some(flag.clone());
                                            let agent = agent.clone(); let events = events.clone();
                                            tests.push(tokio::spawn(async move {
                                                let context=va_runtime::request_metrics(&agent,&RequestOptions::default(),true);
                                                let result = agent.transcribe(audio,context).await.map(|r|super::connected::Transcript {status:r.status.as_str().into(),raw_text:r.raw_text,text:r.text,model_id:r.model_id,stt_backend:r.stt_backend,processing_time_ms:r.processing_time_ms.min(u64::MAX as u128) as u64}).map_err(|e|e.to_string());
                                                if !flag.load(std::sync::atomic::Ordering::Acquire) { let _ = events.send(Event::Test(E::Transcript {request,result})).await; }
                                            }));
                                        }
                                        T::Submit {..} => anyhow::bail!("Web approval belongs to the Web inspector."),
                                    }
                                }
                            }
                            if let Some(id) = &selected { publish(&agent,id,&events,&mut cursor).await?; }
                            Ok(())
                        }.await;
                        if let Err(error) = result { let _ = events.send(Event::Error {action,message:error.to_string()}).await; }
                    }
                    changed = changes.changed() => {
                        if changed.is_err() { break; }
                        if let Some(id) = &selected { if let Err(error) = publish(&agent,id,&events,&mut cursor).await { let _ = events.send(Event::Error {action:"snapshot",message:error.to_string()}).await; } }
                    }
                }
                tests.retain(|task| !task.is_finished());
            }
            if let Some(flag) = test_cancel { flag.store(true,std::sync::atomic::Ordering::Release); }
            agent.shutdown().await;
            for task in tests { task.abort(); }
        }))?;
        Ok(Self {
            sender,
            receiver,
            stop,
            join: Some(join),
        })
    }
    #[cfg(test)]
    pub fn fixture() -> (Self, mpsc::Sender<Event>, mpsc::Receiver<Command>) {
        let (sender, commands) = mpsc::channel(8);
        let (events, receiver) = mpsc::channel(256);
        let (stop, _) = watch::channel(false);
        (
            Self {
                sender,
                receiver,
                stop,
                join: None,
            },
            events,
            commands,
        )
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.sender
            .try_send(command)
            .context("local agent command queue unavailable")
    }
    pub fn event(&mut self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.receiver.close();
        let _ = self.stop.send(true);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::{Duration, Instant};
    use va_core::{
        ConversationConfig, ConversationError, ConversationMessage, ConversationModel, LoadedPrompt,
    };
    #[derive(Default)]
    pub struct Control {
        pub calls: Mutex<Vec<Vec<ConversationMessage>>>,
        pub hold: AtomicBool,
        pub dropped: AtomicUsize,
    }
    struct Model(Arc<Control>);
    impl Drop for Model {
        fn drop(&mut self) {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl ConversationModel for Model {
        fn name(&self) -> &str {
            "local-fake-reply"
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn respond_stream(
            &mut self,
            messages: &[ConversationMessage],
            _: &ConversationConfig,
            cancel: &AtomicBool,
            emit: &mut dyn FnMut(&str),
        ) -> Result<String, ConversationError> {
            self.0.calls.lock().unwrap().push(messages.to_vec());
            while self.0.hold.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) {
                    return Err(ConversationError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            for text in ["Local answer", " — 你好."] {
                emit(text);
            }
            Ok("Local answer — 你好.".into())
        }
    }
    pub fn agent() -> (AgentRuntime, Arc<Control>) {
        let control = Arc::new(Control::default());
        let prompt = LoadedPrompt {
            path: "fixture-role.txt".into(),
            text: "incident role".into(),
            sha256: "f".repeat(64),
        };
        (
            AgentRuntime::new(
                None,
                Some(Box::new(Model(control.clone()))),
                Some(prompt),
                va_runtime::Limits::default(),
                metrics::MetricsConfig::enabled(),
                Arc::new(metrics::MetricsHub::new()),
            ),
            control,
        )
    }
    fn next(connection: &mut Connection, mut matches: impl FnMut(&Event) -> bool) -> Event {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(event) = connection.event() {
                if let Event::Error { message, .. } = &event {
                    panic!("local bridge error: {message}");
                }
                if matches(&event) {
                    return event;
                }
            }
            assert!(Instant::now() < until, "local bridge did not deliver state");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn local_bridge_runs_reply_only_without_services_and_reuses_one_model_for_follow_ups() {
        let (agent, control) = agent();
        let mut connection = Connection::start(agent.clone()).unwrap();
        connection.send(Command::New).unwrap();
        let Event::Created(snapshot) = next(&mut connection, |e| matches!(e, Event::Created(_)))
        else {
            unreachable!()
        };
        let id = snapshot.conversation_id;
        for (key, text) in [
            ("a", "  Incident — café\n你好  "),
            ("b", "Everyone is outside."),
        ] {
            connection
                .send(Command::Text {
                    id: id.clone(),
                    text: text.into(),
                    key: key.into(),
                })
                .unwrap();
            let Event::Snapshot(snapshot) = next(
                &mut connection,
                |e| matches!(e,Event::Snapshot(s) if !s.busy && s.turns.last().is_some_and(|t|t.approved_text.as_deref()==Some(text) && t.status==va_core::chat::ChatPhase::Completed)),
            ) else {
                unreachable!()
            };
            assert_eq!(
                snapshot
                    .runs
                    .iter()
                    .filter(|r| r.stage == "reasoning")
                    .count(),
                snapshot.turns.len()
            );
            assert!(snapshot
                .runs
                .iter()
                .all(|r| r.audio_duration_seconds.is_none()));
        }
        assert_eq!(control.calls.lock().unwrap().len(), 2);
        assert_eq!(control.calls.lock().unwrap()[1].len(), 4);
        assert_eq!(control.dropped.load(Ordering::Relaxed), 0);
        drop(connection);
        drop(agent);
        assert_eq!(control.dropped.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn closing_local_bridge_cancels_and_settles_its_native_work() {
        let (agent, control) = agent();
        control.hold.store(true, Ordering::Release);
        let mut connection = Connection::start(agent.clone()).unwrap();
        connection.send(Command::New).unwrap();
        let Event::Created(snapshot) = next(&mut connection, |e| matches!(e, Event::Created(_)))
        else {
            unreachable!()
        };
        connection
            .send(Command::Text {
                id: snapshot.conversation_id.clone(),
                text: "question".into(),
                key: "a".into(),
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while control.calls.lock().unwrap().is_empty() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(connection);
        let snapshot = conversations::snapshot(&agent, &snapshot.conversation_id, true).unwrap();
        assert!(!snapshot.busy);
        assert_eq!(
            snapshot.turns[0].status,
            va_core::chat::ChatPhase::Cancelled
        );
        drop(agent);
        assert_eq!(control.dropped.load(Ordering::Relaxed), 1);
    }
}
