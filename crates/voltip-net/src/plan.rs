//! What the settings ask for, checked: the system's proxy, none, or one the user sets.

use std::fmt;

use url::Url;

/// The schemes of a proxy the user sets: an HTTP proxy (CONNECT for `https://` and the WebSocket
/// connections, the absolute form for `http://`), SOCKS5 resolving names on this device, SOCKS5
/// resolving them on the proxy.
pub const PROXY_SCHEMES: [&str; 3] = ["http", "socks5", "socks5h"];

/// Longest `no_proxy` list kept.
pub const MAX_NO_PROXY_BYTES: usize = 4096;
/// Longest user name kept: what SOCKS5's sign-in carries (RFC 1929, one length byte).
pub const MAX_USERNAME_BYTES: usize = 255;
/// Longest password kept, for the same reason.
pub const MAX_PASSWORD_BYTES: usize = 255;

/// How Voltip's connections reach the network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProxyPlan {
    /// The system's proxy: the environment (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`,
    /// `NO_PROXY`), then macOS's network settings or Windows' Internet Options; on Android the
    /// network's proxy the shell hands in. None set: direct.
    #[default]
    System,
    /// Direct, whatever the system says.
    Off,
    /// The proxy the user set.
    Manual(ManualProxy),
}

/// A proxy the user set: where it is, how to sign in, which hosts go direct.
#[derive(Clone, PartialEq, Eq)]
pub struct ManualProxy {
    server: Url,
    username: Option<String>,
    password: Option<String>,
    no_proxy: String,
}

impl ManualProxy {
    /// Check and normalise a proxy the user set ([`parse_server`], [`parse_no_proxy`]). A
    /// password without a user name is dropped: there is nothing to send it with.
    pub fn new(server: &str, username: Option<&str>, password: Option<&str>, no_proxy: &str) -> Result<Self, ProxyError> {
        let server = parse_server(server)?;
        let username = username.map(str::trim).filter(|u| !u.is_empty()).map(str::to_owned);
        if username.as_ref().is_some_and(|u| u.len() > MAX_USERNAME_BYTES || u.contains(char::is_control)) {
            return Err(ProxyError::Username);
        }
        let password = username.as_ref().and(password).filter(|p| !p.is_empty()).map(str::to_owned);
        if password.as_ref().is_some_and(|p| p.len() > MAX_PASSWORD_BYTES) {
            return Err(ProxyError::Password);
        }
        Ok(Self { server, username, password, no_proxy: parse_no_proxy(no_proxy)? })
    }

    /// The proxy's address (`scheme://host:port`), without the credentials.
    pub fn server(&self) -> &Url {
        &self.server
    }

    /// The address as the settings keep it: `scheme://host:port`, the port always written out
    /// (the URL leaves out HTTP's 80, which [`parse_server`] would then refuse).
    pub fn address(&self) -> String {
        let port = self.server.port_or_known_default().unwrap_or_default();
        match self.server.host() {
            Some(url::Host::Ipv6(addr)) => format!("{}://[{addr}]:{port}", self.server.scheme()),
            Some(host) => format!("{}://{host}:{port}", self.server.scheme()),
            None => self.server.to_string(),
        }
    }

    /// The user name, if one is set.
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// A password is set.
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// The hosts that go direct, comma-separated ([`parse_no_proxy`]).
    pub fn no_proxy(&self) -> &str {
        &self.no_proxy
    }

    /// The password, for the connection that signs in with it.
    pub(crate) fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }
}

/// The password stays out of logs and panics.
impl fmt::Debug for ManualProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManualProxy")
            .field("server", &self.server.as_str())
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<set>"))
            .field("no_proxy", &self.no_proxy)
            .finish()
    }
}

/// The proxy the platform's network uses, as the shell reads it (Android's `LinkProperties`):
/// an HTTP proxy and the hosts its settings exclude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformProxy {
    pub(crate) server: Url,
    pub(crate) no_proxy: String,
}

impl PlatformProxy {
    /// `host` and `port` of the network's HTTP proxy, and its exclusion list (`*.example.com`,
    /// `example.com`, `10.0.0.0/8` …, separated by commas, semicolons or white space). An entry
    /// this cannot read is left out: the platform accepted it, and the proxy still applies.
    pub fn new(host: &str, port: u16, exclusions: &str) -> Result<Self, ProxyError> {
        let host = host.trim();
        if host.is_empty() || port == 0 {
            return Err(ProxyError::Empty);
        }
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_owned() };
        let readable: Vec<String> =
            exclusions.split([',', ';', ' ', '\t', '\n']).filter_map(|entry| parse_no_proxy(entry).ok()).filter(|e| !e.is_empty()).collect();
        let no_proxy = parse_no_proxy(&readable.join(",")).unwrap_or_default();
        Ok(Self { server: parse_server(&format!("http://{host}:{port}"))?, no_proxy })
    }

    /// The proxy's address.
    pub fn server(&self) -> &Url {
        &self.server
    }
}

/// Why a proxy setting cannot be used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProxyError {
    /// No address.
    #[error("the proxy address is empty")]
    Empty,
    /// Not an address at all.
    #[error("the proxy address is not valid: {0}")]
    Invalid(String),
    /// A scheme other than [`PROXY_SCHEMES`].
    #[error("proxies of type {0} are not supported (http, socks5 and socks5h are)")]
    Scheme(String),
    /// `user:password@` in the address: the password belongs in the keychain.
    #[error("the proxy address carries a user name or password; enter them in their own fields")]
    Credentials,
    /// No port.
    #[error("the proxy address has no port")]
    Port,
    /// A path, query or fragment after the port.
    #[error("the proxy address has a path; give only scheme, host and port")]
    Path,
    /// A `no_proxy` entry that is not a host, domain, address or network.
    #[error("the no-proxy entry {0:?} is not a host, domain, IP address or network")]
    NoProxyEntry(String),
    /// The `no_proxy` list is longer than [`MAX_NO_PROXY_BYTES`].
    #[error("the no-proxy list is too long")]
    NoProxyTooLong,
    /// The user name is longer than [`MAX_USERNAME_BYTES`] or holds control characters.
    #[error("the proxy user name is not valid")]
    Username,
    /// The password is longer than [`MAX_PASSWORD_BYTES`].
    #[error("the proxy password is too long")]
    Password,
}

/// A proxy address as the user typed it, normalised to `scheme://host:port`: `127.0.0.1:7890`
/// is `http://127.0.0.1:7890`; the scheme is one of [`PROXY_SCHEMES`]; the port is required (no
/// proxy is assumed to listen on 80); no credentials, path, query or fragment.
pub fn parse_server(text: &str) -> Result<Url, ProxyError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ProxyError::Empty);
    }
    let with_scheme = if text.contains("://") { text.to_owned() } else { format!("http://{text}") };
    let (scheme, rest) = with_scheme.split_once("://").ok_or_else(|| ProxyError::Invalid(text.to_owned()))?;
    let scheme = scheme.to_ascii_lowercase();
    if !PROXY_SCHEMES.contains(&scheme.as_str()) {
        return Err(ProxyError::Scheme(scheme));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(ProxyError::Credentials);
    }
    if !has_port(authority) {
        return Err(ProxyError::Port);
    }
    let url = Url::parse(&format!("{scheme}://{rest}")).map_err(|e| ProxyError::Invalid(e.to_string()))?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(ProxyError::Invalid(text.to_owned()));
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(ProxyError::Path);
    }
    let port = url.port_or_known_default().ok_or(ProxyError::Port)?;
    let host = url.host_str().unwrap_or_default();
    Url::parse(&format!("{scheme}://{host}:{port}")).map_err(|e| ProxyError::Invalid(e.to_string()))
}

/// `host:port` or `[v6]:port`: the authority names a port.
fn has_port(authority: &str) -> bool {
    let after_host = match authority.rfind(']') {
        Some(end) => &authority[end + 1..],
        None => authority.rfind(':').map_or("", |i| &authority[i..]),
    };
    after_host.strip_prefix(':').is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
}

/// The hosts that go direct, normalised to a comma-separated list: entries separated by commas,
/// semicolons or white space; each a host or domain (it and its subdomains; a leading `*.` or `.`
/// means the same), an IP address, a network (`10.0.0.0/8`), or `*` (everything). Empty: none.
pub fn parse_no_proxy(text: &str) -> Result<String, ProxyError> {
    let mut entries = Vec::new();
    for entry in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|e| !e.is_empty()) {
        let entry = entry.strip_prefix("*.").map_or(entry, |rest| rest);
        let entry = entry.strip_prefix('.').unwrap_or(entry);
        // `[fe80::1]` is the address `fe80::1`: the matcher compares addresses without brackets.
        let entry = entry.strip_prefix('[').and_then(|e| e.strip_suffix(']')).unwrap_or(entry);
        let valid =
            entry == "*" || (!entry.is_empty() && entry.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/' | '[' | ']')));
        if !valid {
            return Err(ProxyError::NoProxyEntry(entry.to_owned()));
        }
        entries.push(entry.to_ascii_lowercase());
    }
    let list = entries.join(",");
    if list.len() > MAX_NO_PROXY_BYTES {
        return Err(ProxyError::NoProxyTooLong);
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_normalised_to_scheme_host_and_port() {
        for (typed, normal) in [
            ("127.0.0.1:7890", "http://127.0.0.1:7890"),
            (" http://proxy.example.com:8080/ ", "http://proxy.example.com:8080"),
            ("HTTP://Proxy.Example.com:3128", "http://proxy.example.com:3128"),
            ("socks5://127.0.0.1:1080", "socks5://127.0.0.1:1080"),
            ("socks5h://gw.example.com:1080", "socks5h://gw.example.com:1080"),
            ("[::1]:8080", "http://[::1]:8080"),
            ("http://localhost:80", "http://localhost"),
        ] {
            assert_eq!(parse_server(typed).map(|u| u.to_string().trim_end_matches('/').to_owned()), Ok(normal.to_owned()), "{typed}");
        }
    }

    #[test]
    fn a_server_that_cannot_be_used_says_why() {
        assert_eq!(parse_server("  "), Err(ProxyError::Empty));
        assert_eq!(parse_server("https://proxy:443"), Err(ProxyError::Scheme("https".into())));
        assert_eq!(parse_server("socks4://proxy:1080"), Err(ProxyError::Scheme("socks4".into())));
        assert_eq!(parse_server("http://user:secret@proxy:8080"), Err(ProxyError::Credentials));
        assert_eq!(parse_server("user@proxy:8080"), Err(ProxyError::Credentials));
        for no_port in ["proxy", "http://proxy", "http://proxy:", "[::1]", "socks5://proxy"] {
            assert_eq!(parse_server(no_port), Err(ProxyError::Port), "{no_port}");
        }
        assert_eq!(parse_server("http://proxy:8080/path"), Err(ProxyError::Path));
        assert_eq!(parse_server("http://proxy:8080?x=1"), Err(ProxyError::Path));
        assert!(matches!(parse_server("http://pro xy:8080"), Err(ProxyError::Invalid(_))));
        assert!(matches!(parse_server("http://:8080"), Err(ProxyError::Invalid(_))));
        assert!(matches!(parse_server("http://proxy:99999"), Err(ProxyError::Invalid(_))));
    }

    #[test]
    fn the_no_proxy_list_is_normalised() {
        assert_eq!(parse_no_proxy(""), Ok(String::new()));
        assert_eq!(
            parse_no_proxy(" localhost, *.Example.com;.corp.test\n10.0.0.0/8  192.168.1.5 ::1 [fe80::1] * "),
            Ok("localhost,example.com,corp.test,10.0.0.0/8,192.168.1.5,::1,fe80::1,*".into())
        );
        assert_eq!(parse_no_proxy("exa mple"), Ok("exa,mple".into()), "white space separates");
        assert_eq!(parse_no_proxy("ex@mple.com"), Err(ProxyError::NoProxyEntry("ex@mple.com".into())));
        assert_eq!(parse_no_proxy("*.*"), Ok("*".into()), "everything");
        assert_eq!(parse_no_proxy(&"a.".repeat(MAX_NO_PROXY_BYTES)), Err(ProxyError::NoProxyTooLong));
    }

    #[test]
    fn a_manual_proxy_keeps_its_credentials_out_of_its_address_and_its_debug() {
        let proxy = ManualProxy::new("socks5://127.0.0.1:1080", Some(" alice "), Some("s3cr:et@"), "corp.test").unwrap();
        assert_eq!(proxy.server().as_str(), "socks5://127.0.0.1:1080");
        assert_eq!((proxy.username(), proxy.has_password(), proxy.no_proxy()), (Some("alice"), true, "corp.test"));
        assert_eq!(proxy.password(), Some("s3cr:et@"));
        let debug = format!("{proxy:?}");
        assert!(!debug.contains("s3cr") && debug.contains("<set>"), "{debug}");
        for typed in ["http://localhost:80", "[::1]:8080", "socks5h://GW.example.com:1080", "127.0.0.1:7890"] {
            let address = ManualProxy::new(typed, None, None, "").unwrap().address();
            assert_eq!(ManualProxy::new(&address, None, None, "").unwrap().address(), address, "{typed} → {address} reads back as itself");
        }
        assert_eq!(ManualProxy::new("http://localhost:80", None, None, "").unwrap().address(), "http://localhost:80");
        assert_eq!(ManualProxy::new("[::1]:8080", None, None, "").unwrap().address(), "http://[::1]:8080");
        let no_user = ManualProxy::new("127.0.0.1:7890", Some("  "), Some("dropped"), "").unwrap();
        assert_eq!((no_user.username(), no_user.has_password()), (None, false), "a password without a user name is dropped");
        assert_eq!(ManualProxy::new("127.0.0.1:7890", Some("a\nb"), None, "").unwrap_err(), ProxyError::Username);
        assert_eq!(ManualProxy::new("127.0.0.1:7890", Some("a"), Some(&"p".repeat(MAX_PASSWORD_BYTES + 1)), "").unwrap_err(), ProxyError::Password);
    }

    #[test]
    fn a_platform_proxy_is_an_http_proxy_with_its_exclusions() {
        let proxy = PlatformProxy::new("10.0.2.2", 8080, "*.corp.test,localhost").unwrap();
        assert_eq!((proxy.server().as_str(), proxy.no_proxy.as_str()), ("http://10.0.2.2:8080/", "corp.test,localhost"));
        assert_eq!(PlatformProxy::new("fe80::1", 3128, "").unwrap().server().as_str(), "http://[fe80::1]:3128/");
        assert_eq!(PlatformProxy::new("proxy", 3128, "*foo, ok.test, b@d").unwrap().no_proxy, "ok.test", "unreadable entries are left out");
        assert_eq!(PlatformProxy::new("", 8080, ""), Err(ProxyError::Empty));
        assert_eq!(PlatformProxy::new("proxy", 0, ""), Err(ProxyError::Empty));
    }
}
