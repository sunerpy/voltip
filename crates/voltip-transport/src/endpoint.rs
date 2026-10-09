//! Where the relay is. Injected, never hard-coded.

use url::Url;

use crate::TransportError;

/// Relay URL used by development builds when nothing is configured. A loopback address:
/// running `voltip-relay` locally is the whole setup.
pub const DEFAULT_DEV_RELAY: &str = "ws://127.0.0.1:47830/ws";

/// A validated relay WebSocket endpoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RelayEndpoint(Url);

impl RelayEndpoint {
    /// Parse and validate: scheme must be `ws` or `wss`, host required.
    pub fn parse(text: &str) -> Result<Self, TransportError> {
        let url = Url::parse(text.trim()).map_err(|e| TransportError::Endpoint(e.to_string()))?;
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(TransportError::Endpoint(format!("scheme must be ws or wss, got {}", url.scheme())));
        }
        if url.host_str().is_none() {
            return Err(TransportError::Endpoint("missing host".into()));
        }
        Ok(Self(url))
    }

    /// Resolve the endpoint for this build: an explicit setting wins, then the build-time
    /// `VOLTIP_RELAY_URL` (injected by CI per environment), then — for debug builds only — the
    /// loopback default. Release builds with nothing configured run **without** a relay.
    pub fn resolve(explicit: Option<&str>) -> Result<Option<Self>, TransportError> {
        if let Some(e) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
            return Self::parse(e).map(Some);
        }
        if let Some(built) = option_env!("VOLTIP_RELAY_URL").map(str::trim).filter(|s| !s.is_empty()) {
            return Self::parse(built).map(Some);
        }
        if cfg!(debug_assertions) {
            return Self::parse(DEFAULT_DEV_RELAY).map(Some);
        }
        Ok(None)
    }

    /// Whether the transport is TLS.
    pub fn is_secure(&self) -> bool {
        self.0.scheme() == "wss"
    }

    /// The URL.
    pub fn url(&self) -> &Url {
        &self.0
    }

    /// Host for display (for example `relay.example.org`).
    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }
}

impl std::fmt::Display for RelayEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (live relay check 2026-09-25): tokio-tungstenite pulls rustls without a crypto
    /// provider, so a binary that did not also link reqwest panicked on the first `wss://` dial
    /// ("Could not automatically determine the process-level CryptoProvider"). The transport crate
    /// now pins `rustls` with exactly one provider; building a client config must not panic here.
    #[test]
    fn regression_wss_dial_has_exactly_one_crypto_provider() {
        // `builder()` resolves the process provider from crate features and panics when zero or two are enabled.
        let config = rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth();
        assert!(!config.crypto_provider().cipher_suites.is_empty());
        assert!(rustls::crypto::CryptoProvider::get_default().is_some(), "builder() installed the single provider as the default");
    }

    #[test]
    fn parses_ws_and_wss_only() {
        let e = RelayEndpoint::parse(" wss://ws.example.app/ws ").unwrap();
        assert!(e.is_secure());
        assert_eq!(e.host(), "ws.example.app");
        assert_eq!(e.to_string(), "wss://ws.example.app/ws");
        assert!(!RelayEndpoint::parse(DEFAULT_DEV_RELAY).unwrap().is_secure());
        assert!(matches!(RelayEndpoint::parse("https://x.example").unwrap_err(), TransportError::Endpoint(_)));
        assert!(matches!(RelayEndpoint::parse("ws://").unwrap_err(), TransportError::Endpoint(_)));
        assert!(matches!(RelayEndpoint::parse("not a url").unwrap_err(), TransportError::Endpoint(_)));
    }

    #[test]
    fn explicit_setting_wins_and_empty_means_fallthrough() {
        let e = RelayEndpoint::resolve(Some("ws://10.0.0.5:1/ws")).unwrap().unwrap();
        assert_eq!(e.host(), "10.0.0.5");
        assert!(RelayEndpoint::resolve(Some("ftp://nope")).is_err());
        // Test builds are debug builds: the loopback default applies unless CI injected a URL.
        let resolved = RelayEndpoint::resolve(Some("   ")).unwrap();
        if option_env!("VOLTIP_RELAY_URL").is_none() {
            assert_eq!(resolved.unwrap().to_string(), DEFAULT_DEV_RELAY);
        } else {
            assert!(resolved.is_some());
        }
    }
}
