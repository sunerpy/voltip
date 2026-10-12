import { describe, expect, it } from "vitest";
import { translate } from "./i18n";
import { MockBackend } from "./mock-backend";
import {
  checkProxySettings,
  parseNoProxy,
  parseProxyServer,
  proxyArgs,
  proxyPasswordProblem,
  proxyProblemCoreText,
  proxyProblemMessage,
} from "./proxy";
import { MAX_NO_PROXY_BYTES, MAX_PROXY_PASSWORD_BYTES, defaultProxySettings } from "./schema";

// The same cases as `voltip_net::plan::tests` and `voltip-core/tests/network.rs`: the form and the
// core agree on what a proxy setting may be.
describe("the proxy form's checks", () => {
  it("tidies a server to scheme://host:port", () => {
    for (const [typed, tidy] of [
      ["127.0.0.1:7890", "http://127.0.0.1:7890"],
      [" http://proxy.example.com:8080/ ", "http://proxy.example.com:8080"],
      ["HTTP://Proxy.Example.com:3128", "http://proxy.example.com:3128"],
      ["socks5://127.0.0.1:1080", "socks5://127.0.0.1:1080"],
      ["socks5h://gw.example.com:1080", "socks5h://gw.example.com:1080"],
      ["[::1]:8080", "http://[::1]:8080"],
      ["http://localhost:80", "http://localhost:80"],
    ] as const) {
      expect([typed, parseProxyServer(typed)]).toEqual([typed, { server: tidy }]);
    }
  });

  it("says why a server cannot be used", () => {
    const code = (text: string) => {
      const parsed = parseProxyServer(text);
      return "problem" in parsed ? parsed.problem.code : undefined;
    };
    expect(code("  ")).toBe("empty");
    expect(parseProxyServer("https://proxy:443")).toEqual({
      problem: { field: "server", code: "scheme", scheme: "https" },
    });
    expect(code("socks4://proxy:1080")).toBe("scheme");
    expect(code("http://user:secret@proxy:8080")).toBe("credentials");
    expect(code("user@proxy:8080")).toBe("credentials");
    for (const noPort of ["proxy", "http://proxy", "http://proxy:", "[::1]", "socks5://proxy"])
      expect([noPort, code(noPort)]).toEqual([noPort, "port"]);
    expect(code("http://proxy:8080/path")).toBe("path");
    expect(code("http://proxy:8080?x=1")).toBe("path");
    expect(code("http://pro xy:8080")).toBe("invalid");
    expect(code("http://:8080")).toBe("invalid");
    expect(code("http://proxy:99999")).toBe("invalid");
  });

  it("tidies the no-proxy list and names the entry it cannot use", () => {
    expect(parseNoProxy("")).toEqual({ noProxy: "" });
    expect(
      parseNoProxy(
        " localhost, *.Example.com;.corp.test\n10.0.0.0/8  192.168.1.5 ::1 [fe80::1] * ",
      ),
    ).toEqual({ noProxy: "localhost,example.com,corp.test,10.0.0.0/8,192.168.1.5,::1,fe80::1,*" });
    expect(parseNoProxy("ex@mple.com")).toEqual({
      problem: { field: "no_proxy", code: "entry", entry: "ex@mple.com" },
    });
    expect(parseNoProxy("*.*")).toEqual({ noProxy: "*" });
    expect(parseNoProxy("a.".repeat(MAX_NO_PROXY_BYTES))).toEqual({
      problem: { field: "no_proxy", code: "too_long" },
    });
  });

  it("checks the whole setting as the core saves it", () => {
    expect(
      checkProxySettings({
        mode: "manual",
        server: " 127.0.0.1:7890 ",
        username: " bob ",
        no_proxy: "*.corp.test; 10.0.0.0/8",
      }),
    ).toEqual({
      proxy: {
        mode: "manual",
        server: "http://127.0.0.1:7890",
        username: "bob",
        no_proxy: "corp.test,10.0.0.0/8",
      },
    });
    expect(checkProxySettings({ ...defaultProxySettings(), mode: "manual" })).toEqual({
      problem: { field: "server", code: "empty" },
    });
    // The other modes keep the manual proxy's fields, checked all the same.
    expect(checkProxySettings({ ...defaultProxySettings(), mode: "off" })).toEqual({
      proxy: { ...defaultProxySettings(), mode: "off" },
    });
    expect(checkProxySettings({ ...defaultProxySettings(), server: "proxy" })).toEqual({
      problem: { field: "server", code: "port" },
    });
    expect(checkProxySettings({ ...defaultProxySettings(), username: "a\nb" })).toEqual({
      problem: { field: "username", code: "invalid" },
    });
  });

  it("checks a new password", () => {
    expect(proxyPasswordProblem("", null)).toBeUndefined();
    expect(proxyPasswordProblem("pw", "bob")).toBeUndefined();
    expect(proxyPasswordProblem("pw", " ")).toEqual({ field: "password", code: "needs_username" });
    expect(proxyPasswordProblem("p".repeat(MAX_PROXY_PASSWORD_BYTES + 1), "bob")).toEqual({
      field: "password",
      code: "too_long",
    });
  });

  it("words each problem in both languages and as the core does", () => {
    const t = (locale: "zh-CN" | "en") => translate.bind(null, locale);
    const problem = { field: "server", code: "scheme", scheme: "https" } as const;
    expect(proxyProblemMessage(problem, t("zh-CN"))).toBe(
      "不支持 https 代理，可以使用 http、socks5、socks5h",
    );
    expect(proxyProblemMessage(problem, t("en"))).toBe(
      "https proxies are not supported; use http, socks5 or socks5h",
    );
    expect(proxyProblemCoreText(problem)).toBe(
      "proxy.server: 不支持 https 代理，可以使用 http、socks5、socks5h",
    );
    expect(proxyProblemCoreText({ field: "no_proxy", code: "entry", entry: "bad@entry" })).toBe(
      "proxy.no_proxy: 「bad@entry」不是有效的主机、域名、IP 地址或网段",
    );
    expect(proxyProblemCoreText({ field: "password", code: "needs_username" })).toBe(
      "proxy.username: 填写密码时也需要填写用户名",
    );
  });
});

describe("settings_set_proxy in the preview", () => {
  it("saves the tidy setting, keeps the password out of the state and reads the system again", async () => {
    const backend = new MockBackend();
    const events: string[] = [];
    backend.on((e) => {
      events.push(e.type);
    });
    await backend.invoke(
      "settings_set_proxy",
      proxyArgs(
        { mode: "manual", server: "127.0.0.1:7890", username: "bob", no_proxy: "*.corp.test" },
        "s3cret",
      ),
    );
    let state = await backend.getState();
    expect(state.settings.proxy).toEqual({
      mode: "manual",
      server: "http://127.0.0.1:7890",
      username: "bob",
      no_proxy: "corp.test",
    });
    expect(state.network.proxy_password).toBe(true);
    expect(JSON.stringify(state)).not.toContain("s3cret");
    expect(backend.savedProxyPassword()).toBe("s3cret");
    // `null` keeps the password; `""` removes it.
    await backend.invoke(
      "settings_set_proxy",
      proxyArgs({ ...state.settings.proxy, mode: "off" }, null),
    );
    expect(backend.savedProxyPassword()).toBe("s3cret");
    await backend.invoke(
      "settings_set_proxy",
      proxyArgs({ ...state.settings.proxy, mode: "off" }, ""),
    );
    state = await backend.getState();
    expect(state.settings.proxy.mode).toBe("off");
    expect(state.network.proxy_password).toBe(false);
    await expect(
      backend.invoke(
        "settings_set_proxy",
        proxyArgs({ ...state.settings.proxy, server: "proxy" }, null),
      ),
    ).rejects.toThrow("proxy.server: 代理地址缺少端口，例如 127.0.0.1:7890");
    backend.setSystemProxy("http://10.0.0.1:3128");
    await backend.invoke("network_refresh");
    expect((await backend.getState()).network.system_proxy).toBe("http://10.0.0.1:3128");
    expect(events.filter((e) => e === "network")).toHaveLength(4);
  });
});
