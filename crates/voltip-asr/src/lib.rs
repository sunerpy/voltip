//! Speech-to-text over an OpenAI-compatible HTTP API (vLLM, OpenAI, any gateway that speaks
//! `POST {base}/v1/audio/transcriptions`).
//!
//! * [`AsrConfig`] — base URL (normalised to end in `/v1`), optional bearer token, model, timeout.
//! * [`AsrClient`] — one reqwest client; [`AsrClient::transcribe`] uploads a WAV as multipart and
//!   returns a [`Transcript`].
//! * [`AsrError`] — HTTP outcomes sorted into what the caller can act on
//!   (`Unauthorized`, `RateLimited`, `Server`, `Network`, `Timeout`, `BadResponse`), with
//!   [`AsrError::is_retryable`].
//!
//! TLS is rustls; nothing here knows a production hostname — the core injects it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod config;
mod error;

pub use client::{AsrClient, Transcript};
pub use config::{AsrConfig, DEFAULT_TIMEOUT, MAX_ERROR_BODY_CHARS, normalize_base_url};
pub use error::AsrError;
