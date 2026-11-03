//! `voltip-relay` binary: `voltip-relay --bind 0.0.0.0:47830`.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use voltip_relay::server::RelayHandle;
use voltip_relay::ticket::TicketKey;
use voltip_relay::{PolicyFiles, RelayConfig};

#[derive(Parser, Debug)]
#[command(name = "voltip-relay", version, about = "Voltip relay: forwards ciphertext between paired devices; never sees plaintext.")]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "VOLTIP_RELAY_BIND", default_value = "127.0.0.1:47830")]
    bind: SocketAddr,
    /// Default pairing-session TTL in seconds.
    #[arg(long, env = "VOLTIP_RELAY_SESSION_TTL", default_value_t = 120)]
    session_ttl: u64,
    /// Emit JSON logs (for container log collectors).
    #[arg(long, env = "VOLTIP_RELAY_JSON_LOGS", default_value_t = false)]
    json_logs: bool,
    /// A reverse proxy whose X-Forwarded-For names the client, for the per-IP limits: an address
    /// or a network; repeat it or separate with commas. Behind nginx on this host: 127.0.0.1.
    #[arg(long = "trust-proxy", env = "VOLTIP_RELAY_TRUST_PROXY", value_delimiter = ',', value_parser = voltip_relay::forwarded::parse_trusted_proxy)]
    trust_proxy: Vec<ipnet::IpNet>,
    /// The policy file the admin console's sync writes (docs/pairing.md 「最低版本」), read again
    /// every 30 s; without it every client version is served.
    #[arg(long, env = "VOLTIP_RELAY_POLICY")]
    policy: Option<PathBuf>,
    /// A policy file written by hand that wins over `--policy` while it exists and is valid.
    #[arg(long = "policy-override", env = "VOLTIP_RELAY_POLICY_OVERRIDE", requires = "policy")]
    policy_override: Option<PathBuf>,
    /// The key that checks device tickets, as hex (docs/pairing.md 「设备票据」). Without it the
    /// relay neither asks for nor reads tickets.
    #[arg(long = "ticket-key-file", env = "VOLTIP_RELAY_TICKET_KEY_FILE")]
    ticket_key_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if args.json_logs {
        tracing_subscriber::fmt().with_env_filter(filter).json().init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    let ticket_key = match &args.ticket_key_file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
            Some(TicketKey::from_hex(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?)
        }
        None => None,
    };
    tracing::info!(tickets = ticket_key.is_some(), "relay starting");
    let config = RelayConfig {
        session_ttl: std::time::Duration::from_secs(args.session_ttl),
        trusted_proxies: args.trust_proxy,
        ticket_key,
        ..RelayConfig::default()
    };
    let handle = RelayHandle::new(config);
    if let Some(policy) = args.policy {
        handle.watch_policy(PolicyFiles::new(policy, args.policy_override));
    }
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown requested");
    };
    let (_addr, task) = handle.serve(args.bind, shutdown).await?;
    task.await.map_err(std::io::Error::other)?
}
