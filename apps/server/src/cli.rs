//! `voltip-server`'s command line (docs/dictation.md §23.5): the service's options, its one-shot
//! actions (`--check`, `--print-token`) and the model library's (`--list-models`,
//! `--download-model`, `--list-compute`).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use voltip_asr_local::LocalDevice;
use voltip_core::serve::{DEFAULT_CONCURRENCY, DEFAULT_PORT, DefaultChoices, EngineOverrides, MAX_CONCURRENCY, MAX_MINUTES};
use voltip_core::{ChineseScript, ProviderId};

/// `voltip-server [OPTIONS]`.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "voltip-server",
    version = crate::APP_VERSION,
    about = "Voltip's local speech service: an OpenAI-compatible transcription endpoint for other programs on this computer.",
    after_help = "Provider keys come from the system keychain the app writes to. Where there is none (a server \
without a desktop session), a key can come from the environment instead, and wins over the keychain: \
VOLTIP_KEY_OPENAI, VOLTIP_KEY_GROQ, VOLTIP_KEY_SILICONFLOW, VOLTIP_KEY_ALIYUN, VOLTIP_KEY_DEEPSEEK, \
VOLTIP_KEY_CUSTOM_ASR, VOLTIP_KEY_CUSTOM_LLM. Prefer a systemd EnvironmentFile readable by your user only; \
the values are never logged.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Where to listen. An address other than this computer's (127.0.0.1, ::1) also needs
    /// --allow-remote.
    #[arg(long, value_name = "ADDR:PORT", default_value_t = SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)))]
    pub listen: SocketAddr,
    /// Allow --listen on an address other computers reach. The service speaks plain HTTP: reach it
    /// over an SSH tunnel or a TLS reverse proxy instead where you can.
    #[arg(long)]
    pub allow_remote: bool,
    /// The bearer token's file (created the first time; default: <data dir>/serve/token).
    #[arg(long, value_name = "PATH")]
    pub token_file: Option<PathBuf>,
    /// The scene every `voltip` request runs with: a built-in category (coding, office, chat,
    /// legal, medical, finance, academic), a scene's name or its id.
    #[arg(long, value_name = "SCENE")]
    pub scene: Option<String>,
    /// Without --scene: run each request with the scene that lists this application id, as if it
    /// were the application in front.
    #[arg(long, value_name = "APP_ID", conflicts_with = "scene")]
    pub app: Option<String>,
    /// The preset every `voltip` request refines with: a built-in one (proofread, prompt, intent,
    /// chat, translate, notes, punctuation, formal), a custom preset's name or its id.
    #[arg(long, value_name = "PRESET")]
    pub preset: Option<String>,
    /// AI polish for every `voltip` request: on or off (default: the settings, or the scene's).
    #[arg(long, value_name = "on|off", value_parser = parse_switch)]
    pub refine: Option<bool>,
    /// The recognition language for every request (zh, en, yue, …; auto = no hint), whatever the
    /// request asks for.
    #[arg(long, value_name = "CODE")]
    pub language: Option<String>,
    /// The script of the Chinese text: simplified, traditional or as_is.
    #[arg(long, value_name = "SCRIPT", value_parser = parse_script)]
    pub script: Option<ChineseScript>,
    /// The speech recognition provider (builtin, local, openai, groq, siliconflow, aliyun, custom).
    #[arg(long, value_name = "PROVIDER", value_parser = parse_provider)]
    pub asr: Option<ProviderId>,
    /// The AI polish provider (builtin, openai, groq, siliconflow, aliyun, deepseek, ollama, custom).
    #[arg(long, value_name = "PROVIDER", value_parser = parse_provider)]
    pub llm: Option<ProviderId>,
    /// Recognise with this local model (see --list-models); implies --asr local.
    #[arg(long, value_name = "ID")]
    pub local_model: Option<String>,
    /// Where local models run: auto, cpu or gpu.
    #[arg(long, value_name = "DEVICE", value_parser = parse_device)]
    pub device: Option<LocalDevice>,
    /// With --device gpu: the GPU by name (see --list-compute).
    #[arg(long, value_name = "NAME")]
    pub gpu: Option<String>,
    /// Inference threads for local models.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=i64::from(voltip_core::MAX_LOCAL_THREADS)))]
    pub threads: Option<u16>,
    /// Longest audio of one request, in minutes.
    #[arg(long, value_name = "MINUTES", default_value_t = MAX_MINUTES, value_parser = clap::value_parser!(u16).range(1..=i64::from(MAX_MINUTES)))]
    pub max_minutes: u16,
    /// Takes processed at the same time (twice as many may wait).
    #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY as u8, value_parser = clap::value_parser!(u8).range(1..=MAX_CONCURRENCY as i64))]
    pub concurrency: u8,
    /// Do not load the local model at start (the first request then waits for it).
    #[arg(long)]
    pub no_preload: bool,
    /// Print the configuration the service would run with, and its warnings, then exit (0: ready).
    #[arg(long, conflicts_with_all = ["print_token", "list_models", "download_model", "list_compute"])]
    pub check: bool,
    /// Print the bearer token (creating it the first time), then exit.
    #[arg(long, conflicts_with_all = ["list_models", "download_model", "list_compute"])]
    pub print_token: bool,
    /// Print the local model library (`id<TAB>state<TAB>name`), then exit.
    #[arg(long, conflicts_with_all = ["download_model", "list_compute"])]
    pub list_models: bool,
    /// Download (or resume) a catalogue model, then exit.
    #[arg(long, value_name = "ID", conflicts_with = "list_compute")]
    pub download_model: Option<String>,
    /// Print what the local models can run on, then exit.
    #[arg(long)]
    pub list_compute: bool,
}

fn parse_switch(text: &str) -> Result<bool, String> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("{other}: expected on or off")),
    }
}

fn parse_script(text: &str) -> Result<ChineseScript, String> {
    [ChineseScript::Simplified, ChineseScript::Traditional, ChineseScript::AsIs]
        .into_iter()
        .find(|s| s.as_str() == text)
        .ok_or_else(|| format!("{text}: expected simplified, traditional or as_is"))
}

fn parse_provider(text: &str) -> Result<ProviderId, String> {
    ProviderId::ALL.into_iter().find(|p| p.as_str() == text).ok_or_else(|| {
        let all: Vec<&str> = ProviderId::ALL.iter().map(|p| p.as_str()).collect();
        format!("{text}: expected one of {}", all.join(", "))
    })
}

fn parse_device(text: &str) -> Result<LocalDevice, String> {
    match text {
        "auto" => Ok(LocalDevice::Auto),
        "cpu" => Ok(LocalDevice::Cpu),
        "gpu" => Ok(LocalDevice::Gpu),
        other => Err(format!("{other}: expected auto, cpu or gpu")),
    }
}

/// How the service runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    /// `--listen`.
    pub listen: SocketAddr,
    /// `--token-file`.
    pub token_file: Option<PathBuf>,
    /// The default processing and the language / script.
    pub choices: DefaultChoices,
    /// The engine overrides.
    pub overrides: EngineOverrides,
    /// `--max-minutes`.
    pub max_minutes: u16,
    /// `--concurrency`.
    pub concurrency: usize,
    /// Not `--no-preload`.
    pub preload: bool,
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run the service until a signal.
    Serve(ServeOptions),
    /// `--check`.
    Check(ServeOptions),
    /// `--print-token`.
    PrintToken {
        /// `--token-file`.
        token_file: Option<PathBuf>,
    },
    /// `--list-models`.
    ListModels,
    /// `--download-model`.
    DownloadModel {
        /// The catalogue id.
        id: String,
    },
    /// `--list-compute`.
    ListCompute,
}

impl Cli {
    /// Parse `argv` (with the program name); a usage error is returned, not printed.
    pub fn parse_args<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::try_parse_from(args)
    }

    /// What to do; the usage error when the options do not go together (exit status 2).
    pub fn action(&self) -> Result<Action, String> {
        if !self.listen.ip().is_loopback()
            && !self.allow_remote
            && !self.list_models
            && self.download_model.is_none()
            && !self.list_compute
            && !self.print_token
        {
            return Err(format!(
                "--listen {} is not this computer's address: add --allow-remote to accept other computers (plain HTTP, the token travels unencrypted)",
                self.listen
            ));
        }
        if self.list_models {
            return Ok(Action::ListModels);
        }
        if let Some(id) = &self.download_model {
            return Ok(Action::DownloadModel { id: id.clone() });
        }
        if self.list_compute {
            return Ok(Action::ListCompute);
        }
        if self.print_token {
            return Ok(Action::PrintToken { token_file: self.token_file.clone() });
        }
        let options = ServeOptions {
            listen: self.listen,
            token_file: self.token_file.clone(),
            choices: DefaultChoices {
                scene: self.scene.clone(),
                app: self.app.clone(),
                preset: self.preset.clone(),
                refine: self.refine,
                language: self.language.clone(),
                script: self.script,
            },
            overrides: EngineOverrides {
                asr: self.asr,
                llm: self.llm,
                local_model: self.local_model.clone(),
                device: self.device,
                gpu: self.gpu.clone(),
                threads: self.threads,
            },
            max_minutes: self.max_minutes,
            concurrency: usize::from(self.concurrency),
            preload: !self.no_preload,
        };
        Ok(if self.check { Action::Check(options) } else { Action::Serve(options) })
    }
}
