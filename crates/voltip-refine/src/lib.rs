//! Dictation post-processing over an OpenAI-compatible chat-completions API (Groq, OpenAI,
//! vLLM, any gateway that speaks `POST {base}/chat/completions`).
//!
//! * [`RefineConfig`] — base URL (normalised to end in `/v1`), optional API key, model, timeout,
//!   [`RefineStyle`].
//! * [`RefineClient`] — one reqwest client; [`RefineClient::refine`] sends [`SYSTEM_PROMPT`] plus
//!   the raw transcript at `temperature 0.2` and returns a cleaned [`Refined`];
//!   [`RefineClient::refine_with`] takes the whole [`PromptHints`]: the take's style and language,
//!   the user's dictionary terms ([`GLOSSARY_CLAUSE`]) and the take's context — app, window title,
//!   scene instruction ([`CONTEXT_CLAUSE`], [`INSTRUCTION_CLAUSE`], docs/dictation.md §18.5).
//! * [`RefineClient::edit`] — the voice edit (docs/dictation.md §19): [`EDIT_SYSTEM_PROMPT`] plus
//!   the selection and the spoken instruction in two blocks tagged with a per-request suffix
//!   ([`edit_user_message`]), `max_tokens` from [`edit_token_budget`], a cut-off answer refused as
//!   [`RefineError::Truncated`], the answer tidied by [`clean_edit_answer`].
//! * [`clean_answer`] — the post-processing (quotes, code fences, line endings), hardware-free.
//! * [`RefineError`] — mirrors `voltip_asr::AsrError`, plus [`RefineError::EmptyAnswer`] for
//!   "the model said nothing", which callers treat as "use the raw text".
//!
//! TLS is rustls; nothing here knows a production hostname or key — the core injects them.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod config;
mod error;
mod prompt;

pub use client::{RefineClient, Refined, clean_answer, clean_edit_answer, edit_token_budget, list_models, output_token_budget};
pub use config::{DEFAULT_TIMEOUT, MAX_ERROR_BODY_CHARS, RefineConfig, RefineStyle, normalize_base_url};
pub use error::RefineError;
pub use prompt::{
    CONTEXT_CLAUSE, EDIT_CONTEXT_CLAUSE, EDIT_GLOSSARY_CLAUSE, EDIT_REMINDER, EDIT_SYSTEM_PROMPT, GLOSSARY_CLAUSE, INSTRUCTION_CLAUSE, MAX_CONTEXT_NAME_CHARS,
    MAX_CONTEXT_TITLE_CHARS, MAX_INSTRUCTION_CHARS, PromptContext, PromptHints, SYSTEM_PROMPT, TEMPERATURE, clean_context_line, edit_nonce, edit_system_prompt,
    edit_user_message, system_prompt,
};
