//! The proxy Voltip's connections go through (docs/dictation.md §24). `Settings.proxy` and the
//! manual proxy's password, kept in the keychain, make the plan of the process's
//! [`voltip_net::Network`] (`CoreConfig::network`), which every client asks for each request. A
//! change rebuilds the engines' clients, whose pooled connections were opened on the old route,
//! and reconnects the relay.

use super::{CoreEvent, Runtime};
use crate::CoreError;
use crate::settings::{PROXY_PASSWORD_ENTRY, ProxySettings, proxy_error_text};
use crate::ui::NetworkStatus;

impl Runtime {
    /// `UiState.network` now.
    fn network_status(&self) -> NetworkStatus {
        let system_proxy = self.config.network.system_https_proxy().map(|url| address(&url));
        NetworkStatus { proxy_password: self.proxy_password.is_some(), system_proxy }
    }

    /// Tell the UI where the network stands (`Ready`, and after every change).
    pub(super) fn emit_network(&mut self) {
        self.network_status = self.network_status();
        self.emit(CoreEvent::Network(self.network_status.clone()));
    }

    /// `SetProxy`: check, keep the password in the keychain, persist, put in force.
    pub(super) fn set_proxy(&mut self, proxy: ProxySettings, password: Option<String>) -> Result<(), CoreError> {
        let invalid = |e: &voltip_net::ProxyError| CoreError::Invalid(proxy_error_text(e));
        let proxy = proxy.checked().map_err(|e| invalid(&e))?;
        let password = match password {
            None => self.proxy_password.clone(),
            Some(new) if new.is_empty() => None,
            Some(new) => {
                if new.len() > voltip_net::MAX_PASSWORD_BYTES {
                    return Err(invalid(&voltip_net::ProxyError::Password));
                }
                if proxy.username.is_none() {
                    return Err(CoreError::Invalid("proxy.username: 填写密码时也需要填写用户名".into()));
                }
                Some(new)
            }
        };
        let plan = proxy.plan(password.as_deref()).map_err(|e| invalid(&e))?;
        if password != self.proxy_password {
            match &password {
                Some(value) => self.secrets.set(PROXY_PASSWORD_ENTRY, value.as_bytes())?,
                None => self.secrets.delete(PROXY_PASSWORD_ENTRY)?,
            }
            self.proxy_password = password;
            tracing::info!(secret = PROXY_PASSWORD_ENTRY, "proxy password updated");
        }
        let changed = plan != self.config.network.plan();
        self.settings.proxy = proxy;
        self.save_settings()?;
        self.config.network.set(plan);
        self.system_fingerprint = self.config.network.system_fingerprint();
        self.emit_network();
        if changed {
            self.reroute()?;
        }
        Ok(())
    }

    /// `RefreshNetwork`: read the system's proxy settings again; when any part of them changed (a
    /// proxy, its credentials, an exclusion; on the phone the network's proxy) while following
    /// them, reconnect what was opened on the old routes.
    pub(super) fn refresh_network(&mut self) -> Result<(), CoreError> {
        let fingerprint = self.config.network.system_fingerprint();
        let changed = fingerprint != self.system_fingerprint;
        self.system_fingerprint = fingerprint;
        self.emit_network();
        if changed && self.config.network.plan() == voltip_net::ProxyPlan::System {
            tracing::info!("the system's proxy settings changed; reconnecting");
            self.reroute()?;
        }
        Ok(())
    }

    /// The route changed: the engines' clients start with empty pools, the relay reconnects.
    fn reroute(&mut self) -> Result<(), CoreError> {
        self.reconfigure_engines();
        self.connect_relay()?;
        self.emit_devices();
        Ok(())
    }
}

/// `scheme://host:port`, the form the settings show.
fn address(url: &url::Url) -> String {
    let port = url.port_or_known_default().map(|p| format!(":{p}")).unwrap_or_default();
    match url.host() {
        Some(url::Host::Ipv6(addr)) => format!("{}://[{addr}]{port}", url.scheme()),
        Some(host) => format!("{}://{host}{port}", url.scheme()),
        None => url.to_string(),
    }
}
