//! Persistent local GGUF weights with isolated native contexts per reply.
//!
//! llama-cpp-2 initializes the backend. Its matched sys binding supplies the
//! native abort callback and length-aware byte APIs absent from the safe wrapper.

use std::ffi::{c_void, CStr, CString};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_sys_2 as native;
use va_core::{
    validate_messages, validate_response, ConversationConfig, ConversationError,
    ConversationMessage, ConversationModel,
};

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend> {
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| anyhow!("could not initialize llama.cpp: {error}"))
}

struct Model(NonNull<native::llama_model>);
// SAFETY: the native model owns its allocation and can move between threads.
// Generation is exclusively accessed through &mut ReplyModel; contexts never
// escape their call or outlive the model. No mutable native state is shared.
unsafe impl Send for Model {}
impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: this is the unique owning pointer from model_load_from_file.
        unsafe { native::llama_model_free(self.0.as_ptr()) };
    }
}

struct NativeContext<'a> {
    ptr: NonNull<native::llama_context>,
    _model: PhantomData<&'a Model>,
    _cancelled: PhantomData<&'a AtomicBool>,
}
impl Drop for NativeContext<'_> {
    fn drop(&mut self) {
        // SAFETY: wait for any asynchronous backend work before freeing the
        // context or releasing its model and borrowed abort-callback data.
        unsafe {
            native::llama_synchronize(self.ptr.as_ptr());
            native::llama_free(self.ptr.as_ptr());
        };
    }
}

struct Sampler(NonNull<native::llama_sampler>);
impl Drop for Sampler {
    fn drop(&mut self) {
        // SAFETY: the chain owns its member samplers and is uniquely owned here.
        unsafe { native::llama_sampler_free(self.0.as_ptr()) };
    }
}

pub struct ReplyModel {
    model: Model,
    template: CString,
    name: String,
    threads: i32,
    load_time_ms: u64,
}

impl ReplyModel {
    pub fn load(
        path: &Path,
        config: &ConversationConfig,
        threads: i32,
        gpu_layers: u32,
    ) -> Result<Self> {
        config.validate()?;
        if threads <= 0 {
            bail!("reply threads must be positive");
        }
        if !path.is_file() {
            bail!("reply model is not a regular file: {}", path.display());
        }
        let started = Instant::now();
        let _backend = backend()?;
        let filename = CString::new(
            path.to_str()
                .ok_or_else(|| anyhow!("reply model path is not UTF-8"))?,
        )?;
        // SAFETY: backend is initialized once and remains alive. The file name
        // stays valid throughout the synchronous native model load.
        let model = unsafe {
            if gpu_layers > 0 && !native::llama_supports_gpu_offload() {
                bail!("this native build has no GPU offload backend; use gpu_layers=0");
            }
            let mut params = native::llama_model_default_params();
            params.n_gpu_layers = i32::try_from(gpu_layers).context("too many GPU layers")?;
            Model(
                NonNull::new(native::llama_model_load_from_file(
                    filename.as_ptr(),
                    params,
                ))
                .ok_or_else(|| anyhow!("llama.cpp could not load {}", path.display()))?,
            )
        };
        // SAFETY: template is owned by the live model; copy it before using it.
        let template = unsafe {
            let ptr = native::llama_model_chat_template(model.0.as_ptr(), std::ptr::null());
            if ptr.is_null() {
                bail!("reply GGUF has no chat template; no generic-role fallback is allowed");
            }
            CStr::from_ptr(ptr).to_owned()
        };
        let mut reply = Self {
            model,
            template,
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("local-gguf")
                .to_owned(),
            threads,
            load_time_ms: 0,
        };
        // Evaluate a tiny prompt at startup to touch weights and initialize the
        // native compute path. This throwaway context cannot leak into any turn.
        let cancelled = AtomicBool::new(false);
        let warmup_prompt = Self::format_messages(
            &reply.template,
            &[
                ConversationMessage {
                    role: va_core::ConversationRole::System,
                    content: "Reply briefly.".into(),
                },
                ConversationMessage {
                    role: va_core::ConversationRole::User,
                    content: "Hello".into(),
                },
            ],
        )?;
        let warmup = reply.tokenize(&warmup_prompt, true)?;
        let context = reply.new_context(config, &cancelled)?;
        reply.decode(&context, &warmup[..warmup.len().min(2)], 0, &cancelled)?;
        drop(context);
        reply.load_time_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        Ok(reply)
    }

    pub fn load_time_ms(&self) -> u64 {
        self.load_time_ms
    }

    fn new_context<'a>(
        &'a self,
        config: &ConversationConfig,
        cancelled: &'a AtomicBool,
    ) -> Result<NativeContext<'a>> {
        // SAFETY: all params are bounded, and the callback borrows cancelled
        // for exactly the context lifetime. The callback only performs an atomic
        // load and never calls user code or unwinds across the native boundary.
        unsafe {
            let trained_context =
                u32::try_from(native::llama_model_n_ctx_train(self.model.0.as_ptr()))
                    .context("model has an invalid training context")?;
            if config.context_size > trained_context {
                bail!("configured reply context exceeds this model's training context");
            }
            let mut params = native::llama_context_default_params();
            params.n_ctx = config.context_size;
            params.n_batch = config.context_size.min(512);
            params.n_ubatch = params.n_batch;
            params.n_threads = self.threads;
            params.n_threads_batch = self.threads;
            params.abort_callback = Some(abort_callback);
            params.abort_callback_data =
                (cancelled as *const AtomicBool).cast_mut().cast::<c_void>();
            let ptr = NonNull::new(native::llama_init_from_model(self.model.0.as_ptr(), params))
                .ok_or_else(|| anyhow!("could not allocate native reply context"))?;
            Ok(NativeContext {
                ptr,
                _model: PhantomData,
                _cancelled: PhantomData,
            })
        }
    }

    fn format_messages(template: &CStr, messages: &[ConversationMessage]) -> Result<String> {
        let strings = messages
            .iter()
            .map(|message| {
                Ok((
                    CString::new(message.role.as_str())?,
                    CString::new(message.content.as_str())?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let chat = strings
            .iter()
            .map(|(role, content)| native::llama_chat_message {
                role: role.as_ptr(),
                content: content.as_ptr(),
            })
            .collect::<Vec<_>>();
        let mut buffer = vec![0u8; 4096];
        loop {
            // SAFETY: every role/content CString and the template outlive this
            // synchronous call; buffer capacity is passed in bytes.
            let written = unsafe {
                native::llama_chat_apply_template(
                    template.as_ptr(),
                    chat.as_ptr(),
                    chat.len(),
                    true,
                    buffer.as_mut_ptr().cast(),
                    i32::try_from(buffer.len())?,
                )
            };
            if written < 0 {
                bail!("GGUF chat template is not supported by llama.cpp");
            }
            let len = written as usize;
            if len > buffer.len() {
                buffer.resize(len, 0);
                continue;
            }
            buffer.truncate(len);
            return String::from_utf8(buffer).context("chat template output is not UTF-8");
        }
    }

    fn tokenize(&self, text: &str, parse_special: bool) -> Result<Vec<native::llama_token>> {
        let text_len =
            i32::try_from(text.len()).context("reply prompt is too large for native tokenizer")?;
        let mut tokens = vec![0; text.len().saturating_add(8)];
        loop {
            // SAFETY: vocabulary belongs to the live model, and both slices
            // have the explicitly supplied lengths. No CString is required.
            let written = unsafe {
                native::llama_tokenize(
                    native::llama_model_get_vocab(self.model.0.as_ptr()),
                    text.as_ptr().cast(),
                    text_len,
                    tokens.as_mut_ptr(),
                    i32::try_from(tokens.len())?,
                    true,
                    parse_special,
                )
            };
            if written == i32::MIN {
                bail!("native tokenization exceeds position range");
            }
            if written < 0 {
                tokens.resize((-written) as usize, 0);
                continue;
            }
            tokens.truncate(written as usize);
            if tokens.is_empty() {
                bail!("native tokenizer returned no prompt tokens");
            }
            return Ok(tokens);
        }
    }

    fn decode(
        &self,
        context: &NativeContext<'_>,
        tokens: &[native::llama_token],
        offset: usize,
        cancelled: &AtomicBool,
    ) -> Result<(), ConversationError> {
        check_cancelled(cancelled)?;
        if tokens.is_empty() {
            return Err(ConversationError::Backend("empty decode batch".into()));
        }
        let mut tokens = tokens.to_vec();
        let mut positions = (offset..offset + tokens.len())
            .map(|pos| pos as i32)
            .collect::<Vec<_>>();
        let mut n_seq_id = vec![1i32; tokens.len()];
        let mut sequence = 0i32;
        let mut sequences = vec![&mut sequence as *mut i32; tokens.len()];
        let mut logits = vec![0i8; tokens.len()];
        *logits.last_mut().expect("non-empty batch") = 1;
        let batch = native::llama_batch {
            n_tokens: tokens.len() as i32,
            token: tokens.as_mut_ptr(),
            embd: std::ptr::null_mut(),
            pos: positions.as_mut_ptr(),
            n_seq_id: n_seq_id.as_mut_ptr(),
            seq_id: sequences.as_mut_ptr(),
            logits: logits.as_mut_ptr(),
        };
        // SAFETY: batch storage and each sequence pointer remain valid through
        // the synchronous decode. Positions are preflighted against context.
        let status = unsafe { native::llama_decode(context.ptr.as_ptr(), batch) };
        check_cancelled(cancelled)?;
        if status != 0 {
            return Err(ConversationError::Backend(format!(
                "native decode failed with status {status}"
            )));
        }
        Ok(())
    }

    fn token_bytes(&self, token: native::llama_token) -> Result<Vec<u8>> {
        let mut bytes = vec![0u8; 64];
        loop {
            // SAFETY: native writes into a length-delimited byte buffer; this
            // intentionally supports NUL bytes and split UTF-8 token pieces.
            let written = unsafe {
                native::llama_token_to_piece(
                    native::llama_model_get_vocab(self.model.0.as_ptr()),
                    token,
                    bytes.as_mut_ptr().cast(),
                    i32::try_from(bytes.len())?,
                    0,
                    false,
                )
            };
            if written == i32::MIN {
                bail!("native token piece exceeds length range");
            }
            if written < 0 {
                bytes.resize((-written) as usize, 0);
                continue;
            }
            bytes.truncate(written as usize);
            return Ok(bytes);
        }
    }
}

fn make_sampler(temperature: f32) -> Result<Sampler> {
    // SAFETY: each sampler is allocated once; chain_add transfers ownership to
    // the chain, which is freed even if a later sampler allocation fails.
    unsafe {
        let sampler = Sampler(
            NonNull::new(native::llama_sampler_chain_init(
                native::llama_sampler_chain_default_params(),
            ))
            .ok_or_else(|| anyhow!("could not allocate sampler chain"))?,
        );
        let add = |ptr| -> Result<()> {
            let ptr =
                NonNull::new(ptr).ok_or_else(|| anyhow!("could not allocate native sampler"))?;
            native::llama_sampler_chain_add(sampler.0.as_ptr(), ptr.as_ptr());
            Ok(())
        };
        if temperature == 0.0 {
            add(native::llama_sampler_init_greedy())?;
        } else {
            add(native::llama_sampler_init_top_k(40))?;
            add(native::llama_sampler_init_top_p(0.95, 1))?;
            add(native::llama_sampler_init_temp(temperature))?;
            // llama.h defines LLAMA_DEFAULT_SEED as 0xFFFFFFFF; bindgen
            // does not export that macro. This requests a native random seed.
            add(native::llama_sampler_init_dist(u32::MAX))?;
        }
        Ok(sampler)
    }
}

unsafe extern "C" fn abort_callback(data: *mut c_void) -> bool {
    // SAFETY: only installed with a valid borrowed AtomicBool. NativeContext's
    // lifetime prevents the callback from outliving that atomic.
    unsafe { (&*data.cast::<AtomicBool>()).load(Ordering::Acquire) }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ConversationError> {
    if cancelled.load(Ordering::Acquire) {
        Err(ConversationError::Cancelled)
    } else {
        Ok(())
    }
}
fn backend_error(error: impl std::fmt::Display) -> ConversationError {
    ConversationError::Backend(error.to_string())
}

impl ConversationModel for ReplyModel {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_ready(&self) -> bool {
        true
    }

    fn respond_stream(
        &mut self,
        messages: &[ConversationMessage],
        config: &ConversationConfig,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(&str),
    ) -> Result<String, ConversationError> {
        check_cancelled(cancelled)?;
        validate_messages(messages, config)?;
        let prompt = Self::format_messages(&self.template, messages).map_err(backend_error)?;
        let tokens = self.tokenize(&prompt, true).map_err(backend_error)?;
        if tokens
            .len()
            .saturating_add(config.max_output_tokens as usize)
            > config.context_size as usize
        {
            return Err(ConversationError::ContextExceeded);
        }
        check_cancelled(cancelled)?;
        let context = self.new_context(config, cancelled).map_err(backend_error)?;
        let batch_size = config.context_size.min(512) as usize;
        for (index, chunk) in tokens.chunks(batch_size).enumerate() {
            self.decode(&context, chunk, index * batch_size, cancelled)?;
        }
        let sampler = make_sampler(config.temperature).map_err(backend_error)?;
        let mut output = Utf8Stream::default();
        for index in 0..config.max_output_tokens as usize {
            check_cancelled(cancelled)?;
            // SAFETY: the context has initialized logits at -1; the sampler is
            // unique to this turn. sample accepts the token into its own state.
            let token = unsafe {
                native::llama_sampler_sample(sampler.0.as_ptr(), context.ptr.as_ptr(), -1)
            };
            // SAFETY: vocabulary belongs to the live model.
            if unsafe {
                native::llama_vocab_is_eog(
                    native::llama_model_get_vocab(self.model.0.as_ptr()),
                    token,
                )
            } {
                break;
            }
            let bytes = self.token_bytes(token).map_err(backend_error)?;
            check_cancelled(cancelled)?;
            output.push(&bytes, config.max_output_chars, emit)?;
            check_cancelled(cancelled)?;
            if index + 1 < config.max_output_tokens as usize {
                self.decode(&context, &[token], tokens.len() + index, cancelled)?;
            }
        }
        check_cancelled(cancelled)?;
        let response = output.finish()?;
        validate_response(&response, config)?;
        Ok(response)
    }
}

#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
    text: String,
    chars: usize,
}
impl Utf8Stream {
    fn push(
        &mut self,
        bytes: &[u8],
        limit: usize,
        emit: &mut dyn FnMut(&str),
    ) -> Result<(), ConversationError> {
        self.pending.extend_from_slice(bytes);
        let valid_len = match std::str::from_utf8(&self.pending) {
            Ok(text) => text.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                return Err(ConversationError::Backend(
                    "reply contains invalid UTF-8".into(),
                ))
            }
        };
        if valid_len == 0 {
            return Ok(());
        }
        let chunk = std::str::from_utf8(&self.pending[..valid_len]).expect("validated prefix");
        if chunk
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(ConversationError::Backend(
                "reply contains invalid control characters".into(),
            ));
        }
        let chars = chunk.chars().count();
        if chars > limit.saturating_sub(self.chars) {
            return Err(ConversationError::OutputTooLong { limit });
        }
        self.chars += chars;
        self.text.push_str(chunk);
        emit(chunk);
        self.pending.drain(..valid_len);
        Ok(())
    }
    fn finish(self) -> Result<String, ConversationError> {
        if !self.pending.is_empty() {
            return Err(ConversationError::Backend(
                "reply ended in incomplete UTF-8".into(),
            ));
        }
        Ok(self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use va_core::build_messages;

    #[test]
    fn native_chat_template_preserves_roles_and_exact_text_without_weights() {
        let config = ConversationConfig::default();
        let question = format!("  {} café\n ", "question ".repeat(600));
        let messages = build_messages(
            "Incident role — only grounded facts.",
            &[],
            &question,
            &config,
        )
        .unwrap();
        let template = CString::new("chatml").unwrap();
        let formatted = ReplyModel::format_messages(&template, &messages).unwrap();
        assert!(formatted.starts_with("<|im_start|>system\nIncident role — only grounded facts."));
        assert!(formatted.contains(&format!("<|im_start|>user\n{question}<|im_end|>")));
        assert!(formatted.ends_with("<|im_start|>assistant\n"));
        assert!(formatted.len() > 4096, "exercises native buffer resizing");
        assert!(ReplyModel::format_messages(
            &CString::new("unsupported-template").unwrap(),
            &messages
        )
        .is_err());
    }

    #[test]
    fn unicode_chunks_reassemble_without_replacement_characters() {
        let mut decoder = Utf8Stream::default();
        let mut chunks = Vec::new();
        for byte in "Hello café 世界 🧯".as_bytes() {
            decoder
                .push(&[*byte], 100, &mut |text| chunks.push(text.to_owned()))
                .unwrap();
        }
        assert_eq!(decoder.finish().unwrap(), "Hello café 世界 🧯");
        assert_eq!(chunks.concat(), "Hello café 世界 🧯");
    }

    #[test]
    fn invalid_utf8_control_and_limits_fail_before_emitting_bad_chunk() {
        let mut emitted = String::new();
        let mut stream = Utf8Stream::default();
        assert!(stream
            .push("你好".as_bytes(), 1, &mut |s| emitted.push_str(s))
            .is_err());
        assert!(emitted.is_empty());
        assert!(Utf8Stream::default()
            .push(&[0xff], 10, &mut |_| {})
            .is_err());
        assert!(Utf8Stream::default().push(b"x\0", 10, &mut |_| {}).is_err());
        let mut incomplete = Utf8Stream::default();
        incomplete.push(&[0xe4], 10, &mut |_| {}).unwrap();
        assert!(incomplete.finish().is_err());
    }

    #[test]
    fn native_abort_callback_reads_shared_atomic() {
        let cancelled = AtomicBool::new(false);
        let data = (&cancelled as *const AtomicBool).cast_mut().cast();
        assert!(!unsafe { abort_callback(data) });
        cancelled.store(true, Ordering::Release);
        assert!(unsafe { abort_callback(data) });
        assert!(matches!(
            check_cancelled(&cancelled),
            Err(ConversationError::Cancelled)
        ));
    }

    #[test]
    fn missing_model_and_invalid_configuration_fail_without_loading_weights() {
        let config = ConversationConfig::default();
        assert!(ReplyModel::load(Path::new("/no/such/reply.gguf"), &config, 1, 0).is_err());
        assert!(ReplyModel::load(Path::new("/no/such/reply.gguf"), &config, 0, 0).is_err());
        fn assert_send<T: Send>() {}
        assert_send::<ReplyModel>();
    }

    #[test]
    #[ignore = "requires PHEME_VA_REPLY_MODEL and PHEME_VA_REPLY_PROMPT; no weights downloaded"]
    fn real_model_stream_cancel_and_fresh_context() {
        let path = std::env::var_os("PHEME_VA_REPLY_MODEL").expect("set PHEME_VA_REPLY_MODEL");
        let prompt_path =
            std::env::var_os("PHEME_VA_REPLY_PROMPT").expect("set PHEME_VA_REPLY_PROMPT");
        let prompt = va_core::load_prompt(Path::new(&prompt_path)).unwrap();
        let config = ConversationConfig {
            temperature: 0.0,
            max_output_tokens: 64,
            ..Default::default()
        };
        let mut model = ReplyModel::load(Path::new(&path), &config, 2, 0).unwrap();
        let cancelled = AtomicBool::new(false);
        let messages = build_messages(
            &prompt.text,
            &[],
            "There is smoke near the east entrance.",
            &config,
        )
        .unwrap();
        let mut chunks = String::new();
        let first = model
            .respond_stream(&messages, &config, &cancelled, &mut |chunk| {
                chunks.push_str(chunk)
            })
            .unwrap();
        assert_eq!(first, chunks);
        assert!(matches!(
            model.respond_stream(&messages, &config, &cancelled, &mut |_| cancelled
                .store(true, Ordering::Release)),
            Err(ConversationError::Cancelled)
        ));
        cancelled.store(false, Ordering::Release);
        let repeated = model
            .respond_stream(&messages, &config, &cancelled, &mut |_| {})
            .unwrap();
        assert_eq!(
            first, repeated,
            "cancelled turn must not contaminate the next fresh context"
        );
    }
}
