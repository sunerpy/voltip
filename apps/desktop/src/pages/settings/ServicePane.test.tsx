import { MOCK_TAKEN_PORTS, MockBackend } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { validPort } from "./ServicePane";

describe("Settings · 本机服务", () => {
  it("switching the service on shows it running at its address; off stops it", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/service" });
    const pane = await screen.findByTestId("service-pane");
    expect(within(pane).getByTestId("service-status")).toHaveTextContent("未启用");
    expect(within(pane).queryByTestId("service-address")).toBeNull();
    await user.click(within(pane).getByRole("switch", { name: "启用本机服务" }));
    await waitFor(() => {
      expect(within(pane).getByTestId("service-status")).toHaveTextContent("运行中");
    });
    expect(within(pane).getByTestId("service-address")).toHaveTextContent(
      "http://127.0.0.1:47840/v1",
    );
    expect(backend.peek().settings.serve).toEqual({ enabled: true, port: 47840 });
    await user.click(within(pane).getByRole("switch", { name: "启用本机服务" }));
    await waitFor(() => {
      expect(within(pane).getByTestId("service-status")).toHaveTextContent("未启用");
    });
  });

  it("a port is applied when valid, refused when not, and a taken one is reported", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/service",
      backend: new MockBackend({ settings: { serve: { enabled: true, port: 47840 } } }),
    });
    const pane = await screen.findByTestId("service-pane");
    const port = within(pane).getByTestId("service-port");
    const apply = within(pane).getByTestId("service-port-apply");
    expect(apply).toBeDisabled();
    await user.clear(port);
    await user.type(port, "80");
    await user.click(apply);
    expect(await within(pane).findByText("端口须在 1024–65535 之间")).toBeInTheDocument();
    expect(backend.peek().settings.serve.port).toBe(47840);
    await user.clear(port);
    await user.type(port, "48000{Enter}");
    await waitFor(() => {
      expect(within(pane).getByTestId("service-address")).toHaveTextContent(
        "http://127.0.0.1:48000/v1",
      );
    });
    const taken = MOCK_TAKEN_PORTS[0] ?? 47999;
    await user.clear(port);
    await user.type(port, `${taken}{Enter}`);
    await waitFor(() => {
      expect(within(pane).getByTestId("service-status")).toHaveTextContent(
        `无法启动：无法监听 127.0.0.1:${taken}`,
      );
    });
    expect(within(pane).queryByTestId("service-address")).toBeNull();
  });

  it("the preset and the scene of other programs' requests are chosen from the app's own", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/service" });
    const pane = await screen.findByTestId("service-pane");
    const preset = within(pane).getByLabelText("预设");
    expect(within(preset).getAllByRole("option")[0]).toHaveTextContent("与全局设置相同");
    expect(within(preset).getByRole("option", { name: "提示词优化" })).toBeInTheDocument();
    await user.selectOptions(preset, "prompt");
    await waitFor(() => {
      expect(backend.peek().settings.serve.preset).toBe("prompt");
    });
    const scene = within(pane).getByLabelText("场景");
    const coding = backend.peek().scenes.find((s) => s.builtin === "coding");
    expect(coding).toBeDefined();
    await user.selectOptions(scene, coding?.id ?? "");
    await waitFor(() => {
      expect(backend.peek().settings.serve.scene).toBe(coding?.id);
    });
    await user.selectOptions(scene, "");
    await waitFor(() => {
      expect(backend.peek().settings.serve.scene).toBeUndefined();
    });
    expect(backend.peek().settings.serve.preset).toBe("prompt");
  });

  it("the token is copied by the core and replaced only after a confirmation; it is never shown", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp({ path: "/settings/service", backend });
    const pane = await screen.findByTestId("service-pane");
    await user.click(within(pane).getByTestId("service-copy-token"));
    await waitFor(() => {
      expect(backend.serveTokenCopies).toBe(1);
    });
    expect(await screen.findByText("令牌已复制到剪贴板")).toBeInTheDocument();
    await user.click(within(pane).getByTestId("service-rotate-token"));
    const confirm = screen.getByRole("dialog", { name: "重新生成令牌？" });
    expect(confirm).toHaveTextContent("旧令牌立即失效");
    expect(backend.serveTokenRotations).toBe(0);
    await user.click(within(confirm).getByRole("button", { name: "重新生成" }));
    await waitFor(() => {
      expect(backend.serveTokenRotations).toBe(1);
    });
    expect(document.body.textContent).not.toMatch(/[0-9a-f]{64}/);
  });

  it("the English pane reads the same", async () => {
    renderApp({ path: "/settings/service", systemLanguage: "en-US" });
    const pane = await screen.findByTestId("service-pane");
    expect(pane).toHaveTextContent("Local service");
    expect(
      within(pane).getByRole("switch", { name: "Turn on the local service" }),
    ).toBeInTheDocument();
    expect(within(pane).getByTestId("service-status")).toHaveTextContent("Off");
  });

  it("only ports from 1024 to 65535 are valid", () => {
    expect([validPort("1024"), validPort(" 47840 "), validPort("65535")]).toEqual([
      1024, 47840, 65535,
    ]);
    for (const bad of ["", "80", "1023", "65536", "47840a", "-1", "1e4"])
      expect(validPort(bad)).toBeUndefined();
  });
});
