//! The only process in a combined Whisper/reply deployment that links llama.cpp.
//! stdout is exclusively versioned NDJSON; native diagnostics go to stderr.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use anyhow::{anyhow, bail, Context, Result};
use reply_model::protocol::{self, Command, Event};
use va_core::{ConversationConfig, ConversationError, ConversationMessage, ConversationModel};

struct Job {
    id: u64,
    messages: Vec<ConversationMessage>,
    config: ConversationConfig,
    cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
struct InputState {
    active: Option<(u64, Arc<AtomicBool>)>,
    last_id: u64,
    stopped: bool,
}

fn main() {
    if let Err(error) = run(
        BufReader::new(std::io::stdin()),
        BufWriter::new(std::io::stdout()),
    ) {
        eprintln!("pheme-reply-worker: {error}");
        std::process::exit(1);
    }
}

fn run(mut input: impl BufRead + Send + 'static, mut output: impl Write) -> Result<()> {
    let Some(Command::Init {
        protocol: version,
        path,
        config,
        threads,
        gpu_layers,
    }) = protocol::read_frame(&mut input)?
    else {
        bail!("first reply worker command must be init");
    };
    let loaded = (|| {
        if version != protocol::VERSION {
            bail!("unsupported reply IPC protocol version");
        }
        protocol::validate_config(&config)?;
        reply_native::ReplyModel::load(&path, &config, threads, gpu_layers)
    })();
    let mut model = match loaded {
        Ok(model) => model,
        Err(error) => {
            protocol::write_frame(
                &mut output,
                &Event::StartupError {
                    message: error.to_string(),
                },
            )?;
            return Err(error);
        }
    };
    protocol::write_frame(
        &mut output,
        &Event::Ready {
            protocol: protocol::VERSION,
            name: model.name().to_owned(),
            load_time_ms: model.load_time_ms(),
        },
    )?;
    let state = Arc::new(Mutex::new(InputState::default()));
    let (jobs_tx, jobs) = mpsc::sync_channel(1);
    let reader_state = Arc::clone(&state);
    let reader = thread::spawn(move || {
        let result = read_commands(&mut input, &jobs_tx, &reader_state);
        stop_input(&reader_state);
        result
    });
    while let Ok(job) = jobs.recv() {
        if state
            .lock()
            .map_err(|_| anyhow!("reply input state poisoned"))?
            .stopped
        {
            break;
        }
        let mut output_error = None;
        let result =
            model.respond_stream(&job.messages, &job.config, &job.cancelled, &mut |text| {
                if output_error.is_none() {
                    if let Err(error) = protocol::write_frame(
                        &mut output,
                        &Event::Delta {
                            id: job.id,
                            text: text.to_owned(),
                        },
                    ) {
                        output_error = Some(error);
                        job.cancelled.store(true, Ordering::Release);
                    }
                }
            });
        if let Some(error) = output_error {
            return Err(error.into());
        }
        // Clear before sending the terminal frame: the parent may immediately
        // submit its next turn upon reading that frame. The job still owns the
        // cancellation atomic until inference and all callbacks have settled.
        state
            .lock()
            .map_err(|_| anyhow!("reply input state poisoned"))?
            .active = None;
        let result = if job.cancelled.load(Ordering::Acquire) {
            Err(ConversationError::Cancelled)
        } else {
            result
        };
        let terminal = match result {
            Ok(text) => Event::Done { id: job.id, text },
            Err(error) => Event::Error {
                id: job.id,
                error: error.into(),
            },
        };
        protocol::write_frame(&mut output, &terminal)?;
    }
    // Shutdown/EOF cancels and settles active inference before dropping weights.
    reader
        .join()
        .map_err(|_| anyhow!("reply input reader panicked"))??;
    Ok(())
}

fn stop_input(state: &Mutex<InputState>) {
    if let Ok(mut state) = state.lock() {
        state.stopped = true;
        if let Some((_, cancelled)) = &state.active {
            cancelled.store(true, Ordering::Release);
        }
    }
}

fn read_commands(
    input: &mut impl BufRead,
    jobs: &mpsc::SyncSender<Job>,
    state: &Mutex<InputState>,
) -> Result<()> {
    while let Some(command) = protocol::read_frame(input)? {
        match command {
            Command::Generate {
                id,
                messages,
                config,
            } => {
                protocol::validate_config(&config)?;
                va_core::validate_messages(&messages, &config)?;
                let cancelled = Arc::new(AtomicBool::new(false));
                {
                    let mut state = state
                        .lock()
                        .map_err(|_| anyhow!("reply input state poisoned"))?;
                    if state.active.is_some() || id == 0 || id <= state.last_id {
                        bail!("overlapping or non-monotonic reply worker request");
                    }
                    // Register in the reader, not the inference thread, so an
                    // immediate cancel cannot race ahead of a queued generate.
                    state.active = Some((id, Arc::clone(&cancelled)));
                    state.last_id = id;
                }
                jobs.try_send(Job {
                    id,
                    messages,
                    config,
                    cancelled,
                })
                .context("reply inference queue unavailable")?;
            }
            Command::Cancel { id } => {
                let state = state
                    .lock()
                    .map_err(|_| anyhow!("reply input state poisoned"))?;
                if let Some((active_id, cancelled)) = &state.active {
                    if *active_id == id {
                        cancelled.store(true, Ordering::Release);
                    }
                }
            }
            Command::Shutdown => break,
            Command::Init { .. } => bail!("reply worker may only initialize once"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn generate(id: u64) -> Command {
        Command::Generate {
            id,
            messages: va_core::build_messages(
                "role",
                &[],
                "question",
                &ConversationConfig::default(),
            )
            .unwrap(),
            config: ConversationConfig::default(),
        }
    }
    fn input(commands: &[Command]) -> Cursor<Vec<u8>> {
        Cursor::new(
            commands
                .iter()
                .flat_map(|command| protocol::encode_frame(command).unwrap())
                .collect(),
        )
    }

    #[test]
    fn cancel_reaches_the_atomic_before_queued_inference_starts() {
        let (tx, rx) = mpsc::sync_channel(1);
        let state = Mutex::new(InputState::default());
        read_commands(
            &mut input(&[generate(1), Command::Cancel { id: 1 }]),
            &tx,
            &state,
        )
        .unwrap();
        let job = rx.try_recv().unwrap();
        assert!(job.cancelled.load(Ordering::Acquire));
        assert_eq!(state.lock().unwrap().last_id, 1);
    }

    #[test]
    fn stale_cancel_does_not_cancel_another_turn_and_overlap_is_rejected() {
        let (tx, rx) = mpsc::sync_channel(1);
        let state = Mutex::new(InputState::default());
        read_commands(
            &mut input(&[generate(2), Command::Cancel { id: 1 }]),
            &tx,
            &state,
        )
        .unwrap();
        let job = rx.try_recv().unwrap();
        assert!(!job.cancelled.load(Ordering::Acquire));
        assert!(read_commands(&mut input(&[generate(3)]), &tx, &state).is_err());
        stop_input(&state);
        assert!(job.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn completed_request_ids_cannot_be_replayed() {
        let (tx, rx) = mpsc::sync_channel(1);
        let state = Mutex::new(InputState::default());
        read_commands(&mut input(&[generate(4)]), &tx, &state).unwrap();
        rx.try_recv().unwrap();
        state.lock().unwrap().active = None;
        assert!(read_commands(&mut input(&[generate(4)]), &tx, &state).is_err());
    }
}
