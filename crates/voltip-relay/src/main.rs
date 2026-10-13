//! `voltip-relay` binary: `voltip-relay --bind 0.0.0.0:47830`.

use std::net::SocketAddr;

use clap::Parser;
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;

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
    let config = RelayConfig { session_ttl: std::time::Duration::from_secs(args.session_ttl), trusted_proxies: args.trust_proxy, ..RelayConfig::default() };
    let handle = RelayHandle::new(config);
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown requested");
    };
    let (_addr, task) = handle.serve(args.bind, shutdown).await?;
    task.await.map_err(std::io::Error::other)?
}
