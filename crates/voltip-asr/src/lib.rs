//! Speech-to-text over an OpenAI-compatible HTTP API (vLLM, OpenAI, any gateway that speaks
//! `POST {base}/v1/audio/transcriptions`), and over Alibaba Cloud Model Studio's own protocols.
//!
//! * [`AsrConfig`] — base URL (normalised to end in `/v1`), optional bearer token, model, timeout.
//! * [`AsrClient`] — one reqwest client; [`AsrClient::transcribe`] uploads a WAV as multipart and
//!   returns a [`Transcript`].
//! * [`DashscopeClient`] — Model Studio (docs/dictation.md §3.4): the chat-completions form of
//!   `qwen3-asr-flash`, the native multimodal generation of `qwen-audio-…-asr-flash`, and the
//!   realtime models' WebSocket task protocol ([`duplex`]), live or with a whole take.
//! * [`AsrError`] — outcomes sorted into what the caller can act on (`Unauthorized`,
//!   `RateLimited`, `Server`, `Service`, `FreeQuotaExhausted`, `Network`, `Timeout`,
//!   `BadResponse`, …), with [`AsrError::is_retryable`].
//!
//! TLS is rustls; nothing here knows a production hostname — the core injects it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod config;
mod dashscope;
pub mod duplex;
mod error;

pub use client::{AsrClient, Transcript};
pub use config::{AsrConfig, DEFAULT_TIMEOUT, MAX_ERROR_BODY_CHARS, normalize_base_url};
pub use dashscope::{DashscopeClient, DashscopeMode, HOTWORD_WEIGHT, MAX_DATA_URI_BYTES, MAX_HOTWORDS, compatible_base, origin_of};
pub use error::AsrError;
