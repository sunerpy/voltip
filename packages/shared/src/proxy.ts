// The proxy setting's pure helpers (docs/dictation.md §24), shared by the desktop's 网络 pane and
// the phone's 代理 page: the checks the core makes before it saves a manual proxy
// (`voltip_net::parse_server`, `parse_no_proxy`), so a form can say what is wrong before it sends
// anything, the tidy form the core saves, and the arguments of `settings_set_proxy`. The core
// checks everything again.
import type { TFunction } from "./i18n";
import {
  MAX_NO_PROXY_BYTES,
  MAX_PROXY_PASSWORD_BYTES,
  MAX_PROXY_USERNAME_BYTES,
  PROXY_SCHEMES,
  type ProxySettings,
  type SetProxyArgs,
} from "./schema";

/** What 不使用代理的地址 starts with for a first manual proxy: the local network, so a model
 *  service on another computer of the network is not sent to a proxy that cannot reach it. This
 *  device's own addresses go direct whatever the list says. */
export const DEFAULT_NO_PROXY = "local, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16";

/** Why a manual proxy cannot be saved: the field and what is wrong with it. */
export type ProxyProblem =
  | {
      field: "server";
      code: "empty" | "invalid" | "scheme" | "credentials" | "port" | "path";
      scheme?: string;
    }
  | { field: "no_proxy"; code: "entry"; entry: string }
  | { field: "no_proxy"; code: "too_long" }
  | { field: "username"; code: "invalid" }
  | { field: "password"; code: "too_long" | "needs_username" };

const utf8 = new TextEncoder();

/** `host:port` or `[v6]:port`: the authority names a port. */
function hasPort(authority: string): boolean {
  const bracket = authority.lastIndexOf("]");
  const colon = authority.lastIndexOf(":");
  const afterHost =
    bracket >= 0 ? authority.slice(bracket + 1) : colon >= 0 ? authority.slice(colon) : "";
  return /^:\d+$/.test(afterHost);
}

/** A proxy address as typed, tidied to `scheme://host:port` (`127.0.0.1:7890` is HTTP), or why it
 *  cannot be used: the scheme is one of `PROXY_SCHEMES`, the port is required, and there are no
 *  credentials, path, query or fragment. */
export function parseProxyServer(text: string): { server: string } | { problem: ProxyProblem } {
  const problem = (code: Extract<ProxyProblem, { field: "server" }>["code"], scheme?: string) => ({
    problem: { field: "server" as const, code, ...(scheme === undefined ? {} : { scheme }) },
  });
  const trimmed = text.trim();
  if (trimmed === "") return problem("empty");
  const withScheme = trimmed.includes("://") ? trimmed : `http://${trimmed}`;
  const split = withScheme.indexOf("://");
  const scheme = withScheme.slice(0, split).toLowerCase();
  if (!(PROXY_SCHEMES as readonly string[]).includes(scheme)) return problem("scheme", scheme);
  const rest = withScheme.slice(split + 3);
  const authority = rest.split(/[/?#]/, 1)[0] ?? "";
  if (authority.includes("@")) return problem("credentials");
  if (!hasPort(authority)) return problem("port");
  let url: URL;
  try {
    // The host is checked as an HTTP URL's: what the core's URL parser takes, near enough.
    url = new URL(`http://${authority}`);
  } catch {
    return problem("invalid");
  }
  if (url.hostname === "") return problem("invalid");
  const after = rest.slice(authority.length);
  if (after !== "" && after !== "/") return problem("path");
  const port = Number(authority.slice(authority.lastIndexOf(":") + 1));
  return { server: `${scheme}://${url.hostname}:${port}` };
}

/** The hosts that go direct, tidied to a comma-separated list (entries separated by commas,
 *  semicolons or white space; `*.example.com` and `.example.com` mean `example.com` and its
 *  subdomains; `*` means everything), or the entry that is not a host, domain, address or network. */
export function parseNoProxy(text: string): { noProxy: string } | { problem: ProxyProblem } {
  const entries: string[] = [];
  for (const raw of text.split(/[,;\s]+/).filter((e) => e !== "")) {
    let entry = raw.startsWith("*.") ? raw.slice(2) : raw;
    entry = entry.startsWith(".") ? entry.slice(1) : entry;
    if (entry.startsWith("[") && entry.endsWith("]")) entry = entry.slice(1, -1);
    const valid = entry === "*" || /^[A-Za-z0-9.\-_:/[\]]+$/.test(entry);
    if (!valid) return { problem: { field: "no_proxy", code: "entry", entry } };
    entries.push(entry.toLowerCase());
  }
  const noProxy = entries.join(",");
  if (noProxy.length > MAX_NO_PROXY_BYTES)
    return { problem: { field: "no_proxy", code: "too_long" } };
  return { noProxy };
}

/** `settings` checked and tidied as the core saves them (`ProxySettings::checked`): the manual
 *  mode needs a server; the others keep the manual proxy's fields, checked all the same. */
export function checkProxySettings(
  settings: ProxySettings,
): { proxy: ProxySettings } | { problem: ProxyProblem } {
  const username = settings.username?.trim() ?? "";
  if (utf8.encode(username).length > MAX_PROXY_USERNAME_BYTES || /\p{Cc}/u.test(username))
    return { problem: { field: "username", code: "invalid" } };
  const noProxy = parseNoProxy(settings.no_proxy);
  if ("problem" in noProxy) return noProxy;
  let server: string | null = null;
  if ((settings.server ?? "").trim() !== "") {
    const parsed = parseProxyServer(settings.server ?? "");
    if ("problem" in parsed) return parsed;
    server = parsed.server;
  } else if (settings.mode === "manual") {
    return { problem: { field: "server", code: "empty" } };
  }
  return {
    proxy: {
      mode: settings.mode,
      server,
      username: username === "" ? null : username,
      no_proxy: noProxy.noProxy,
    },
  };
}

/** What is wrong with a new password for `username` (an empty one removes the saved password). */
export function proxyPasswordProblem(
  password: string,
  username: string | null,
): ProxyProblem | undefined {
  if (password === "") return undefined;
  if (utf8.encode(password).length > MAX_PROXY_PASSWORD_BYTES)
    return { field: "password", code: "too_long" };
  if ((username ?? "").trim() === "") return { field: "password", code: "needs_username" };
  return undefined;
}

/** `settings_set_proxy` for `proxy`; `password`: a new one, `""` to remove it, `null` to keep it. */
export function proxyArgs(proxy: ProxySettings, password: string | null): SetProxyArgs {
  return {
    mode: proxy.mode,
    server: proxy.server,
    username: proxy.username,
    noProxy: proxy.no_proxy,
    password,
  };
}

/** What a form says about `problem`. */
export function proxyProblemMessage(problem: ProxyProblem, t: TFunction): string {
  switch (problem.field) {
    case "server":
      switch (problem.code) {
        case "empty":
          return t("settings.network.problem.serverEmpty");
        case "invalid":
          return t("settings.network.problem.serverInvalid");
        case "scheme":
          return t("settings.network.problem.serverScheme", { scheme: problem.scheme ?? "" });
        case "credentials":
          return t("settings.network.problem.serverCredentials");
        case "port":
          return t("settings.network.problem.serverPort");
        case "path":
          return t("settings.network.problem.serverPath");
      }
      return t("settings.network.problem.serverInvalid");
    case "no_proxy":
      return problem.code === "entry"
        ? t("settings.network.problem.noProxyEntry", { entry: problem.entry })
        : t("settings.network.problem.noProxyTooLong");
    case "username":
      return t("settings.network.problem.usernameInvalid");
    case "password":
      return problem.code === "too_long"
        ? t("settings.network.problem.passwordTooLong")
        : t("settings.network.problem.passwordNeedsUsername");
  }
}

/** The core's text for a server problem. */
const SERVER_CORE_TEXT: Record<Extract<ProxyProblem, { field: "server" }>["code"], string> = {
  empty: "手动代理需要填写代理地址",
  invalid: "代理地址无效",
  scheme: "不支持 {scheme} 代理，可以使用 http、socks5、socks5h",
  credentials: "地址中不要包含用户名或密码，请填写在用户名和密码中",
  port: "代理地址缺少端口，例如 127.0.0.1:7890",
  path: "代理地址只需要协议、主机和端口",
};

/** The core's text for `problem` (`voltip_core::proxy_error_text`), for the preview's backend. */
export function proxyProblemCoreText(problem: ProxyProblem): string {
  switch (problem.field) {
    case "server":
      return `proxy.server: ${SERVER_CORE_TEXT[problem.code].replace("{scheme}", problem.scheme ?? "")}`;
    case "no_proxy":
      return problem.code === "entry"
        ? `proxy.no_proxy: 「${problem.entry}」不是有效的主机、域名、IP 地址或网段`
        : "proxy.no_proxy: 不使用代理的地址过多";
    case "username":
      return "proxy.username: 用户名无效";
    case "password":
      return problem.code === "too_long"
        ? "proxy.password: 密码过长"
        : "proxy.username: 填写密码时也需要填写用户名";
  }
}
