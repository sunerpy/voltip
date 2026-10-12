import { MockBackend } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

describe("Settings · 网络", () => {
  it("follows the system by default and shows the system's proxy, read again on request", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ network: { proxy_password: false } });
    renderApp({ path: "/settings/network", backend });
    const pane = await screen.findByTestId("network-pane");
    expect(within(pane).getByRole("radio", { name: "跟随系统" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(within(pane).getByTestId("network-system-proxy")).toHaveTextContent("未设置");
    expect(within(pane).getByText("系统未设置代理，直接连接。")).toBeInTheDocument();
    backend.setSystemProxy("http://127.0.0.1:7890");
    await user.click(within(pane).getByTestId("network-refresh"));
    await waitFor(() => {
      expect(within(pane).getByTestId("network-system-proxy")).toHaveTextContent(
        "http://127.0.0.1:7890",
      );
    });
    expect(backend.peek().settings.proxy.mode).toBe("system");
  });

  it("no proxy applies at once; a manual proxy is checked, saved tidy, its password kept out of view", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/network" });
    const pane = await screen.findByTestId("network-pane");
    await user.click(within(pane).getByRole("radio", { name: "不使用代理" }));
    await waitFor(() => {
      expect(backend.peek().settings.proxy.mode).toBe("off");
    });

    // 手动设置 without a saved server waits for one.
    await user.click(within(pane).getByRole("radio", { name: "手动设置" }));
    expect(within(pane).getByTestId("network-pending")).toHaveTextContent(
      "填写代理服务器并保存后生效。",
    );
    expect(backend.peek().settings.proxy.mode).toBe("off");
    const server = within(pane).getByTestId("network-server");
    await user.type(server, "127.0.0.1");
    await user.click(within(pane).getByTestId("network-save"));
    expect(await within(pane).findByText("请填写端口，例如 127.0.0.1:7890")).toBeInTheDocument();
    expect(backend.peek().settings.proxy.mode).toBe("off");
    await user.type(server, ":7890");
    await user.type(within(pane).getByTestId("network-password"), "s3cret");
    expect(await within(pane).findByText("填写密码时也需要填写用户名")).toBeInTheDocument();
    await user.type(within(pane).getByTestId("network-username"), " bob ");
    await user.type(within(pane).getByTestId("network-no-proxy"), "*.corp.test; 10.0.0.0/8");
    await user.click(within(pane).getByTestId("network-save"));
    await waitFor(() => {
      expect(backend.peek().settings.proxy).toEqual({
        mode: "manual",
        server: "http://127.0.0.1:7890",
        username: "bob",
        no_proxy: "corp.test,10.0.0.0/8",
      });
    });
    expect(backend.savedProxyPassword()).toBe("s3cret");
    expect(await screen.findByText("代理设置已保存")).toBeInTheDocument();
    const password = within(pane).getByTestId("network-password");
    await waitFor(() => {
      expect(password).toHaveValue("");
    });
    expect(password).toHaveAttribute("placeholder", "已保存，输入新密码可替换");
    expect(within(pane).getByTestId("network-server")).toHaveValue("http://127.0.0.1:7890");
    expect(document.body.textContent).not.toContain("s3cret");
    expect(JSON.stringify(backend.peek())).not.toContain("s3cret");

    // Following the system again keeps the manual proxy for later, and its password.
    await user.click(within(pane).getByRole("radio", { name: "跟随系统" }));
    await waitFor(() => {
      expect(backend.peek().settings.proxy.mode).toBe("system");
    });
    expect(backend.peek().settings.proxy.server).toBe("http://127.0.0.1:7890");
    await user.click(within(pane).getByRole("radio", { name: "手动设置" }));
    await waitFor(() => {
      expect(backend.peek().settings.proxy.mode).toBe("manual");
    });
    expect(backend.savedProxyPassword()).toBe("s3cret");
    await user.click(within(pane).getByTestId("network-password-clear"));
    await waitFor(() => {
      expect(backend.savedProxyPassword()).toBeUndefined();
    });
    expect(within(pane).queryByTestId("network-password-clear")).toBeNull();
  });

  it("an unsupported scheme and a bad no-proxy entry are named before anything is sent", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/network" });
    const pane = await screen.findByTestId("network-pane");
    await user.click(within(pane).getByRole("radio", { name: "手动设置" }));
    await user.type(within(pane).getByTestId("network-server"), "https://127.0.0.1:8443");
    await user.click(within(pane).getByTestId("network-save"));
    expect(
      await within(pane).findByText("不支持 https 代理，可以使用 http、socks5、socks5h"),
    ).toBeInTheDocument();
    await user.clear(within(pane).getByTestId("network-server"));
    await user.type(within(pane).getByTestId("network-server"), "socks5h://127.0.0.1:1080");
    await user.type(within(pane).getByTestId("network-no-proxy"), "ok.test, b@d");
    expect(
      await within(pane).findByText("「b@d」不是有效的主机、域名、IP 地址或网段"),
    ).toBeInTheDocument();
    expect(backend.peek().settings.proxy.mode).toBe("system");
  });

  it("regression: the English interface names the modes and problems in English", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/settings/network", systemLanguage: "en-US" });
    const pane = await screen.findByTestId("network-pane");
    expect(within(pane).getByRole("radio", { name: "Follow the system" })).toBeInTheDocument();
    await user.click(within(pane).getByRole("radio", { name: "Manual" }));
    await user.type(within(pane).getByTestId("network-server"), "proxy");
    await user.click(within(pane).getByTestId("network-save"));
    expect(
      await within(pane).findByText("Add the port, such as 127.0.0.1:7890"),
    ).toBeInTheDocument();
  });
});
