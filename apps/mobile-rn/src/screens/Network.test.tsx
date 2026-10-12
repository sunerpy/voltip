// 设置 › 网络 on the phone (docs/dictation.md §24): the network's own proxy by default, none, or one
// set here, checked as the core checks it; the password never comes back to the page.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

async function openNetwork() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-network"));
  return screen.findByTestId("phone-network");
}

async function chooseMode(label: "system" | "off" | "manual") {
  await fireEvent.press(await screen.findByTestId("network-mode"));
  await fireEvent.press(await screen.findByTestId(`network-mode-option-${label}`));
}

describe("the phone's network page", () => {
  it("follows the network's proxy by default and shows it, read again when it changes", async () => {
    const backend = new MockBackend({ role: "phone", network: { proxy_password: false } });
    await renderApp({ backend });
    await openTab("settings");
    expect(await screen.findByTestId("settings-network")).toHaveTextContent(/跟随系统/);
    const page = await openNetwork();
    expect(within(page).getByText("使用当前网络（Wi-Fi）的代理设置。")).toBeOnTheScreen();
    expect(within(page).getByText("未设置")).toBeOnTheScreen();
    await act(async () => {
      backend.setSystemProxy("http://10.0.2.2:8080");
      await backend.invoke("network_refresh");
    });
    expect(await within(page).findByText("http://10.0.2.2:8080")).toBeOnTheScreen();
  });

  it("a manual proxy is checked, saved tidy, and its password stays out of the page", async () => {
    const { backend } = await renderApp();
    const page = await openNetwork();
    await chooseMode("manual");
    const manual = await within(page).findByTestId("network-manual");
    expect(within(manual).getByTestId("network-no-proxy")).toHaveProp(
      "value",
      "local, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16",
    );
    expect(within(manual).getByText("填写代理服务器并保存后生效。")).toBeOnTheScreen();
    await fireEvent.changeText(within(manual).getByTestId("network-server"), "127.0.0.1");
    await fireEvent.press(within(manual).getByTestId("network-save"));
    expect(await within(manual).findByText("请填写端口，例如 127.0.0.1:7890")).toBeOnTheScreen();
    expect(backend.peek().settings.proxy.mode).toBe("system");
    await fireEvent.changeText(
      within(manual).getByTestId("network-server"),
      "socks5h://127.0.0.1:1080",
    );
    await fireEvent.changeText(within(manual).getByTestId("network-username"), "bob");
    await fireEvent.changeText(within(manual).getByTestId("network-password"), "s3cret");
    await fireEvent.changeText(within(manual).getByTestId("network-no-proxy"), "*.corp.test");
    await fireEvent.press(within(manual).getByTestId("network-save"));
    await waitFor(() => {
      expect(backend.peek().settings.proxy).toEqual({
        mode: "manual",
        server: "socks5h://127.0.0.1:1080",
        username: "bob",
        no_proxy: "corp.test",
      });
    });
    expect(backend.savedProxyPassword()).toBe("s3cret");
    await waitFor(() => {
      expect(within(manual).getByTestId("network-password")).toHaveProp("value", "");
    });
    expect(within(manual).getByTestId("network-password")).toHaveProp(
      "placeholder",
      "已保存，输入新密码可替换",
    );
    expect(JSON.stringify(backend.peek())).not.toContain("s3cret");
    await fireEvent.press(within(manual).getByTestId("network-password-clear"));
    await waitFor(() => {
      expect(backend.savedProxyPassword()).toBeUndefined();
    });
    // 不使用代理 applies at once; the manual proxy stays for later.
    await chooseMode("off");
    await waitFor(() => {
      expect(backend.peek().settings.proxy.mode).toBe("off");
    });
    expect(backend.peek().settings.proxy.server).toBe("socks5h://127.0.0.1:1080");
  });
});
