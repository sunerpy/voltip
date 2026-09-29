import {
  type InjectPreflight,
  PERMISSION_POLL_INTERVAL_MS,
  type PermissionReport,
  defaultEngineSettings,
} from "@voltip/shared";
import {
  MOCK_ASR_MS,
  MOCK_DICTATION_DWELL_MS,
  MOCK_DICTATION_TEXT,
  MOCK_HOTKEY_BACKEND,
  MOCK_REFINE_MS,
  MockBackend,
  desktopIdentity,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MACOS_BUNDLE_ID, MACOS_TCC_RESET, MACOS_TCC_RESET_MICROPHONE } from "./Onboarding";
import { engineSettingsFor, initialChoice, vendorsFor } from "./OnboardingEngine";

const mac = () => ({ ...desktopIdentity(), platform: "macos" as const, name: "MacBook Pro" });

const macReport = (overrides: Partial<PermissionReport> = {}): PermissionReport => ({
  platform: "macos",
  microphone: "granted",
  accessibility: "granted",
  ...overrides,
});

/** Replace `permissionsStatus` with reads the test settles one by one, so the poll count never
 *  depends on how fast the runner is. */
function deferredReads(backend: MockBackend) {
  const reads: { resolve: (r: PermissionReport) => void; reject: (e: unknown) => void }[] = [];
  backend.permissionsStatus = () =>
    new Promise<PermissionReport>((resolve, reject) => {
      reads.push({ resolve, reject });
    });
  return reads;
}

/** The status cell of one permission row (`data-state` carries the wire state). */
async function permissionCell(id: string, state: string) {
  const table = await screen.findByRole("table", { name: "系统权限" });
  await waitFor(() => {
    expect(within(table).getByTestId(`permission-${id}`)).toHaveAttribute("data-state", state);
  });
  return within(table).getByTestId(`permission-${id}`);
}

describe("Onboarding wizard", () => {
  it("step 1 shows the live macOS permission states, the unsigned banner and greys the sidebar", async () => {
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: () => Promise.resolve() },
      configurable: true,
    });
    // The TCC-reset banner is macOS-only, so this identity is a Mac.
    renderApp({
      path: "/onboarding",
      mock: {
        identity: mac(),
        permissions: macReport({ accessibility: "denied", microphone: "not_determined" }),
      },
    });
    expect(await screen.findByRole("heading", { name: "系统权限", level: 2 })).toBeInTheDocument();
    // The identity arrives with core_state, after the first paint of the step.
    expect(await screen.findByText("权限 · macOS")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "设置向导 / 第 1 步", level: 1 }),
    ).toBeInTheDocument();
    expect(await permissionCell("accessibility", "denied")).toHaveTextContent("已拒绝");
    const table = screen.getByRole("table", { name: "系统权限" });
    expect(within(table).getByText("辅助功能")).toBeInTheDocument();
    expect(within(table).getByTestId("permission-microphone")).toHaveTextContent("尚未询问");
    // A header row plus exactly the two permissions the platform crate reports; no Input
    // Monitoring (regression, public release 2026-09-27: no trigger needs it).
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(within(table).queryByTestId("permission-input_monitoring")).toBeNull();
    // Only the two that are not granted offer a request.
    expect(within(table).getAllByRole("button", { name: "请求授权" })).toHaveLength(2);
    expect(screen.getByRole("button", { name: /历史记录/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /首页/ })).toBeEnabled();
    // Both grants the last ad-hoc → fixed-certificate update may lose (plan 1.7), each with its copy.
    const commands = screen.getByTestId("tcc-reset-commands");
    expect(
      within(commands).getByText("tccutil reset Accessibility dev.voltip.desktop"),
    ).toBeInTheDocument();
    expect(
      within(commands).getByText("tccutil reset Microphone dev.voltip.desktop"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "复制辅助功能的重置命令" }));
    expect(await screen.findByText("已复制修复命令")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "复制麦克风的重置命令" })).toBeInTheDocument();
    expect(screen.getByTestId("permission-hint")).toHaveTextContent(
      "未授予辅助功能权限，文本无法插入到光标处，只会保存在历史记录中。",
    );
    expect(screen.getByRole("button", { name: "继续" })).toBeDisabled();
  });

  it("regression: step 1 has no sample rows or disabled deep links any more; every row is a live state and 重新检查 reads again", async () => {
    const user = userEvent.setup();
    let reads = 0;
    renderApp({
      path: "/onboarding",
      mock: {
        permissions: () => {
          reads += 1;
          return {
            platform: "windows",
            microphone: "granted",
            accessibility: "not_applicable",
          };
        },
      },
    });
    expect(await permissionCell("microphone", "granted")).toHaveTextContent("已授权");
    const table = screen.getByRole("table", { name: "系统权限" });
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(within(table).getByTestId("permission-accessibility")).toHaveTextContent("本平台不适用");
    expect(within(table).queryByRole("button", { name: "打开系统设置" })).toBeNull();
    expect(within(table).queryByRole("button", { name: "请求授权" })).toBeNull();
    const recheck = screen.getByRole("button", { name: "重新检查" });
    expect(recheck).toBeEnabled();
    expect(recheck).not.toHaveAttribute("title");
    expect(screen.getByTestId("permission-poll")).toHaveTextContent("每秒自动检查");
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
    expect(document.body.textContent).not.toMatch(/第二阶段|示例数据|尚未接入/);
    const before = reads;
    await user.click(recheck);
    await waitFor(() => {
      expect(reads).toBeGreaterThan(before);
    });
    expect(screen.getByRole("button", { name: "继续" })).toBeEnabled();
  });

  it("Linux: nothing to grant, every row reads not applicable and 继续 goes straight on", async () => {
    const user = userEvent.setup();
    renderApp({
      path: "/onboarding",
      mock: { identity: { ...desktopIdentity(), platform: "linux", name: "ThinkPad" } },
    });
    expect(await screen.findByText("权限 · Linux")).toBeInTheDocument();
    expect(await screen.findByTestId("nothing-to-grant")).toHaveTextContent(
      "Linux 上没有需要授权的项目，可以直接继续。",
    );
    const table = screen.getByRole("table", { name: "系统权限" });
    for (const id of ["microphone", "accessibility"]) {
      expect(within(table).getByTestId(`permission-${id}`)).toHaveAttribute(
        "data-state",
        "not_applicable",
      );
      expect(within(table).getByTestId(`permission-${id}`)).toHaveTextContent("本平台不适用");
    }
    expect(within(table).queryByRole("button")).toBeNull();
    expect(screen.queryByTestId("permission-hint")).toBeNull();
    expect(screen.queryByText(/tccutil/)).toBeNull();
    const next = screen.getByRole("button", { name: "继续" });
    expect(next).toBeEnabled();
    await user.click(next);
    expect(screen.getByRole("heading", { name: "快捷键", level: 2 })).toBeInTheDocument();
  });

  it("macOS: a denied microphone and Accessibility block 继续 until each request comes back granted", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/onboarding",
      mock: {
        identity: mac(),
        permissions: macReport({ microphone: "denied", accessibility: "denied" }),
      },
    });
    await permissionCell("microphone", "denied");
    const next = screen.getByRole("button", { name: "继续" });
    expect(next).toBeDisabled();
    // The microphone is named first: without it nothing can be recorded at all.
    expect(screen.getByTestId("permission-hint")).toHaveTextContent("麦克风未授权，无法录音");
    expect(next).toHaveAttribute("title", expect.stringContaining("麦克风未授权"));
    const table = screen.getByRole("table", { name: "系统权限" });
    const micRequest = within(
      within(table).getByTestId("permission-microphone").closest("tr") ?? table,
    ).getByRole("button", { name: "请求授权" });
    await user.click(micRequest);
    await permissionCell("microphone", "granted");
    expect(screen.getByTestId("permission-hint")).toHaveTextContent(
      "未授予辅助功能权限，文本无法插入到光标处，只会保存在历史记录中。",
    );
    expect(screen.getByRole("button", { name: "继续" })).toBeDisabled();
    await user.click(within(table).getByRole("button", { name: "请求授权" }));
    await permissionCell("accessibility", "granted");
    expect(backend.permissionRequests).toEqual(["microphone", "accessibility"]);
    expect(screen.queryByTestId("permission-hint")).toBeNull();
    await user.click(screen.getByRole("button", { name: "继续" }));
    expect(screen.getByRole("heading", { name: "快捷键", level: 2 })).toBeInTheDocument();
  });

  it("three consecutive failed reads stop the one-second poll; 重新检查 resumes it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const backend = new MockBackend({ now: () => Date.now(), identity: mac() });
      const reads = deferredReads(backend);
      renderApp({ path: "/onboarding", backend });
      await screen.findByRole("table", { name: "系统权限" });
      await waitFor(() => {
        expect(reads).toHaveLength(1);
      });
      const poll = screen.getByTestId("permission-poll");
      for (let n = 1; n <= 2; n += 1) {
        await act(async () => {
          reads[n - 1]?.reject(new Error("TCC 查询超时"));
          await Promise.resolve();
        });
        expect(poll).toHaveAttribute("data-stopped", "false");
        expect(screen.queryByText("无法读取权限状态")).toBeNull();
        await act(async () => {
          await vi.advanceTimersByTimeAsync(PERMISSION_POLL_INTERVAL_MS);
        });
        expect(reads).toHaveLength(n + 1);
      }
      await act(async () => {
        reads[2]?.reject(new Error("TCC 查询超时"));
        await Promise.resolve();
      });
      expect(poll).toHaveAttribute("data-stopped", "true");
      expect(poll).toHaveTextContent("连续 3 次读取失败 · 已停止自动检查");
      expect(screen.getByText("无法读取权限状态")).toBeInTheDocument();
      expect(screen.getByText(/系统查询失败：TCC 查询超时/)).toBeInTheDocument();
      // Stopped means stopped: no read however long the step stays open.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(PERMISSION_POLL_INTERVAL_MS * 5);
      });
      expect(reads).toHaveLength(3);
      // The header button and the banner's both restart the loop.
      const rechecks = screen.getAllByRole("button", { name: "重新检查" });
      expect(rechecks).toHaveLength(2);
      const bannerRecheck = rechecks.at(-1);
      expect(bannerRecheck).toBeDefined();
      if (bannerRecheck) await user.click(bannerRecheck);
      await waitFor(() => {
        expect(reads).toHaveLength(4);
      });
      expect(poll).toHaveAttribute("data-stopped", "false");
      await act(async () => {
        reads[3]?.resolve(macReport());
        await Promise.resolve();
      });
      expect(await permissionCell("accessibility", "granted")).toHaveTextContent("已授权");
      expect(screen.queryByText("无法读取权限状态")).toBeNull();
      expect(poll).toHaveTextContent("每秒自动检查");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(PERMISSION_POLL_INTERVAL_MS);
      });
      expect(reads).toHaveLength(5);
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: step 1 names the running platform, not the macOS fixture, and hides the macOS-only TCC banner elsewhere", async () => {
    renderApp({ path: "/onboarding" });
    expect(await screen.findByRole("heading", { name: "系统权限", level: 2 })).toBeInTheDocument();
    // The identity arrives with core_state, after the first paint of the step.
    expect(await screen.findByText("权限 · Windows")).toBeInTheDocument();
    expect(screen.queryByText(/macOS 15/)).toBeNull();
    expect(screen.queryByText(/tccutil/)).toBeNull();
  });

  it("step 2 records both hotkey edges (or skips) and reaches the engine step", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/onboarding?step=2" });
    const monitor = await screen.findByTestId("edge-monitor");
    expect(monitor).toHaveAttribute("data-edges", "waiting");
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: " ", code: "Space", ctrlKey: true, altKey: true }),
      );
    });
    expect(monitor).toHaveAttribute("data-edges", "pressed");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keyup", { key: " ", code: "Space" }));
    });
    expect(monitor).toHaveAttribute("data-edges", "passed");
    expect(screen.getByText("已检测 2/2")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "继续" }));
    expect(screen.getByRole("heading", { name: "选择语音模型", level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /内置服务/ })).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("button", { name: "上一步" }));
    expect(screen.getByRole("heading", { name: "快捷键", level: 2 })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "跳过测试" }));
    expect(screen.getByTestId("edge-monitor")).toHaveAttribute("data-edges", "skipped");
  });

  it("regression: step 2 reports the saved chord, the running platform and the shell's backend instead of the macOS fixture, and the system hotkey's own edges pass the monitor", async () => {
    const { backend } = renderApp({ path: "/onboarding?step=2" });
    const monitor = await screen.findByTestId("edge-monitor");
    expect(screen.getByTestId("onboarding-hotkey-backend")).toHaveTextContent("Windows");
    expect(screen.getByTestId("onboarding-hotkey-backend")).toHaveTextContent(MOCK_HOTKEY_BACKEND);
    expect(screen.queryByText(/carbon/)).toBeNull();
    expect(screen.queryByText(/macOS 15/)).toBeNull();
    expect(screen.getByTestId("onboarding-hotkey-status")).toHaveTextContent("已在系统中生效");
    // A real registration failure replaces the reassurance.
    act(() => {
      backend.publish({
        type: "hotkey",
        error: "Ctrl+Alt+Space 注册失败：HotKey already registered",
        pressed: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    expect(screen.getByTestId("onboarding-hotkey-status")).toHaveTextContent(
      "HotKey already registered",
    );
    // The system is named once, without the library or the system call behind it.
    expect(screen.getByTestId("onboarding-hotkey-backend")).toHaveTextContent(/^Windows$/);
    // Press and release reported by the OS-level hotkey, not by this window.
    act(() => {
      backend.publish({
        type: "hotkey",
        registered: "Ctrl+Alt+Space",
        pressed: true,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    expect(monitor).toHaveAttribute("data-edges", "pressed");
    act(() => {
      backend.publish({
        type: "hotkey",
        registered: "Ctrl+Alt+Space",
        pressed: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
      });
    });
    expect(monitor).toHaveAttribute("data-edges", "passed");
  });

  it("step 3 offers the built-in service first, writes the choice and the polish switch through settings_set_engines and moves on", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/onboarding?step=3" });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    const group = screen.getByRole("radiogroup", { name: "识别服务" });
    expect(
      within(group)
        .getAllByRole("radio")
        .map((r) => r.querySelector("span span")?.firstChild?.textContent),
    ).toEqual(["内置服务", "本地识别（离线）", "其他服务商"]);
    const builtin = within(group).getByRole("radio", { name: /内置服务/ });
    expect(builtin).toHaveAttribute("aria-checked", "true");
    expect(builtin).toHaveTextContent("Qwen3-ASR-1.7B · 开箱即用，无需密钥");
    expect(builtin).toHaveTextContent("推荐");
    // No host anywhere on the step, no skip link, no phase talk.
    expect(document.body.textContent).not.toMatch(/voltip\.example|第二阶段|示例数据|下载并继续/);
    expect(screen.queryByRole("button", { name: /跳过/ })).toBeNull();
    // Polish is on and done by the built-in service; switch it off and save.
    const refine = screen.getByRole("switch", { name: "同时用大模型润色" });
    expect(refine).toBeChecked();
    expect(screen.getByText(/服务商：内置服务/)).toBeInTheDocument();
    await user.click(refine);
    await user.click(screen.getByRole("button", { name: "保存并继续" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines).toMatchObject({
        asr_provider: "builtin",
        llm_provider: "builtin",
        refine_enabled: false,
      });
    });
    expect(screen.getByRole("heading", { name: "试说一句", level: 2 })).toBeInTheDocument();
    expect(screen.getByTestId("trial-note")).toHaveTextContent("不润色");
    expect(screen.getByTestId("trial-note")).toHaveTextContent("内置服务");
  });

  it("regression: another provider needs its key, which goes to provider_key_set and is never echoed", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/onboarding?step=3" });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    await user.click(screen.getByRole("radio", { name: /其他服务商/ }));
    const form = screen.getByTestId("onboarding-provider");
    const provider = within(form).getByLabelText("服务商");
    expect(
      within(provider)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["OpenAI", "Groq", "硅基流动", "自定义接口"]);
    await user.selectOptions(provider, "groq");
    const next = screen.getByRole("button", { name: "保存并继续" });
    expect(next).toBeDisabled();
    expect(screen.getByText("这个服务商需要 API 密钥")).toBeInTheDocument();
    const key = within(form).getByLabelText("API 密钥");
    expect(key).toHaveAttribute("type", "password");
    await user.type(key, "gsk_secret_42");
    await user.selectOptions(within(form).getByLabelText("模型"), "whisper-large-v3");
    expect(next).toBeEnabled();
    // Groq polishes too: the switch names it.
    expect(screen.getByText(/服务商：Groq/)).toBeInTheDocument();
    await user.click(next);
    await waitFor(() => {
      expect(backend.peek().settings.engines).toMatchObject({
        asr_provider: "groq",
        llm_provider: "groq",
        refine_enabled: true,
        providers: { groq: { asr_model: "whisper-large-v3" } },
      });
    });
    expect(backend.peek().engines).toMatchObject({
      asr_ready: true,
      asr_model: "whisper-large-v3",
      asr_host: "api.groq.com",
    });
    expect(JSON.stringify(backend.peek())).not.toContain("gsk_secret_42");
    expect(document.body.textContent).not.toContain("gsk_secret_42");
    expect(screen.getByTestId("trial-note")).toHaveTextContent("whisper-large-v3 · Groq");
  });

  it("a custom endpoint needs an http(s) address; its key is optional", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/onboarding?step=3" });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    await user.click(screen.getByRole("radio", { name: /其他服务商/ }));
    const form = screen.getByTestId("onboarding-provider");
    await user.selectOptions(within(form).getByLabelText("服务商"), "custom");
    const next = screen.getByRole("button", { name: "保存并继续" });
    expect(next).toBeDisabled();
    expect(screen.getByText("自定义接口需要以 http:// 或 https:// 开头的地址")).toBeInTheDocument();
    await user.type(within(form).getByLabelText("接口地址"), "asr.corp.local");
    expect(next).toBeDisabled();
    await user.clear(within(form).getByLabelText("接口地址"));
    await user.type(within(form).getByLabelText("接口地址"), "https://asr.corp.local/v1");
    await user.type(within(form).getByLabelText("模型"), "whisper-large-v3");
    expect(next).toBeEnabled();
    await user.click(next);
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom).toEqual({
        asr_model: "whisper-large-v3",
        asr_url: "https://asr.corp.local/v1",
      });
    });
    expect(backend.peek().engines.asr_provider).toBe("custom");
    expect(backend.peek().engines.asr_host).toBe("asr.corp.local");
  });

  it("regression: a custom endpoint without a model cannot be saved; the core would refuse every take", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/onboarding?step=3" });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    await user.click(screen.getByRole("radio", { name: /其他服务商/ }));
    const form = screen.getByTestId("onboarding-provider");
    await user.selectOptions(within(form).getByLabelText("服务商"), "custom");
    await user.type(within(form).getByLabelText("接口地址"), "https://asr.corp.local/v1");
    const next = screen.getByRole("button", { name: "保存并继续" });
    expect(next).toBeDisabled();
    expect(screen.getByText("请填写模型名称")).toBeInTheDocument();
    await user.type(within(form).getByLabelText("模型"), "   ");
    expect(next).toBeDisabled();
    await user.type(within(form).getByLabelText("模型"), "whisper-large-v3");
    expect(next).toBeEnabled();
    expect(screen.queryByText("请填写模型名称")).not.toBeInTheDocument();
  });

  it("regression: reopening the guide shows the saved custom endpoint, and saving keeps it instead of erasing it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ now: () => 1_758_700_000_000 });
    const saved = { asr_model: "whisper-large-v3", asr_url: "https://asr.corp.local/v1" };
    await backend.invoke("settings_set_engines", {
      engines: {
        ...backend.peek().settings.engines,
        asr_provider: "custom",
        providers: { custom: saved },
      },
    });
    renderApp({ path: "/onboarding?step=3", backend });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    const form = screen.getByTestId("onboarding-provider");
    expect(within(form).getByLabelText("接口地址")).toHaveValue(saved.asr_url);
    expect(within(form).getByLabelText("模型")).toHaveValue(saved.asr_model);
    // An emptied model field would drop the saved one: the custom endpoint has no default.
    await user.clear(within(form).getByLabelText("模型"));
    const next = screen.getByRole("button", { name: "保存并继续" });
    expect(next).toBeDisabled();
    await user.type(within(form).getByLabelText("模型"), saved.asr_model);
    expect(next).toBeEnabled();
    await user.click(next);
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom).toEqual(saved);
    });
  });

  it("the on-device choice shows the recommended model with its download button and writes the local provider", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/onboarding?step=3" });
    await screen.findByRole("heading", { name: "选择语音模型", level: 2 });
    await user.click(screen.getByRole("radio", { name: /本地识别/ }));
    const local = screen.getByTestId("onboarding-local");
    const card = within(local).getByRole("article", { name: "均衡" });
    expect(within(card).getByRole("button", { name: "下载" })).toBeEnabled();
    expect(within(local).getByText(/下载完成后才能听写/)).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "下载" }));
    await waitFor(() => {
      expect(backend.peek().models[0]?.state.kind).not.toBe("not_installed");
    });
    await user.click(screen.getByRole("button", { name: "保存并继续" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.asr_provider).toBe("local");
    });
    expect(backend.peek().engines.local_model).toBe("qwen3-asr-0.6b");
  });

  it("engineSettingsFor keeps the rest of the block and only rewrites what the choice owns", () => {
    const engines = new MockBackend().peek().engines;
    const current = {
      ...defaultEngineSettings(),
      local_model: "sense-voice-small",
      live_preview: false,
      output_mode: "streaming_final" as const,
      vad_trim: true,
      chinese_script: "traditional" as const,
      refine_enabled: false,
      inject: "clipboard_only" as const,
      language: "zh",
    };
    const draft = { provider: "groq" as const, model: "", baseUrl: "", key: "", refine: true };
    expect(engineSettingsFor("builtin", current, draft, engines)).toEqual({
      ...current,
      asr_provider: "builtin",
      llm_provider: "builtin",
      refine_enabled: true,
    });
    expect(engineSettingsFor("local", current, { ...draft, refine: false }, engines)).toEqual({
      ...current,
      asr_provider: "local",
      llm_provider: "builtin",
      refine_enabled: false,
    });
    expect(
      engineSettingsFor("provider", current, { ...draft, model: " whisper-large-v3 " }, engines),
    ).toEqual({
      ...current,
      asr_provider: "groq",
      llm_provider: "groq",
      refine_enabled: true,
      providers: { groq: { asr_model: "whisper-large-v3" } },
    });
    // A build without the built-in service: on-device recognition has no polish provider.
    const bare = new MockBackend({ builtIn: {} }).peek().engines;
    expect(engineSettingsFor("local", current, draft, bare)).toEqual({
      ...current,
      asr_provider: "local",
      refine_enabled: false,
    });
    expect(initialChoice(bare)).toBe("local");
    expect(vendorsFor(bare)).toEqual(["openai", "groq", "siliconflow", "custom"]);
  });

  it("regression: the trial step runs a real dictation and shows the inserted text; 完成设置 is always available", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({
        path: "/onboarding?step=4",
        backend: new MockBackend({ now: () => Date.now() }),
      });
      const finish = await screen.findByRole("button", { name: "完成设置" });
      expect(finish).toBeEnabled();
      expect(screen.queryByRole("button", { name: /跳过/ })).toBeNull();
      expect(screen.queryByTestId("deferred-badge")).toBeNull();
      expect(document.body.textContent).not.toMatch(/第二阶段|示例数据/);
      expect(screen.getByRole("meter", { name: "强度" })).toHaveAttribute("aria-valuenow", "0");
      expect(screen.getByText("就绪")).toBeInTheDocument();
      const box = screen.getByRole("textbox", { name: "在这里试说" });
      expect(box).toHaveValue("");
      await user.click(screen.getByRole("button", { name: "试说一句" }));
      expect(backend.peek().dictation.phase.phase).toBe("listening");
      expect(screen.getByTestId("trial-status")).toHaveAttribute("data-phase", "listening");
      expect(screen.getByText(/正在录音… 00:0\d/)).toBeInTheDocument();
      // The meter is on while the recorder is open.
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      await user.click(screen.getByRole("button", { name: "停止" }));
      expect(screen.getByText("识别中…")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "处理中…" })).toBeDisabled();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS + MOCK_REFINE_MS);
      });
      expect(box).toHaveValue(MOCK_DICTATION_TEXT);
      expect(screen.getByTestId("trial-note")).toHaveTextContent(
        "听到了 · 已润色 · 文本也已按「粘贴」送到当时光标所在的位置。",
      );
      expect(screen.getByText(/已插入 \d+ 字 · 粘贴 · 已润色/)).toBeInTheDocument();
      expect(backend.activeMeters()).toBe(0);
      // The core goes idle after its dwell; the result stays in the box and the button offers another go.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByText("就绪")).toBeInTheDocument();
      expect(box).toHaveValue(MOCK_DICTATION_TEXT);
      expect(screen.getByRole("button", { name: "再说一句" })).toBeEnabled();
      await user.click(screen.getByRole("button", { name: "完成设置" }));
      expect(await screen.findByText("设置向导已完成")).toBeInTheDocument();
      expect(screen.getAllByRole("heading", { name: "首页", level: 1 }).length).toBeGreaterThan(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("the trial step can be cancelled mid-recording", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/onboarding?step=4" });
    await user.click(await screen.findByRole("button", { name: "试说一句" }));
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
    expect(screen.getByText("已取消")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "在这里试说" })).toHaveValue("");
  });

  it("step 4 names the Windows injection preflight; hosts that do not check show nothing", async () => {
    const checked = (
      decision: InjectPreflight["decision"],
      process: string | null,
    ): InjectPreflight => ({
      platform: "windows",
      checked: true,
      decision,
      target_process: process,
      self_level: "medium",
      target_level: decision === "elevated_target" ? "high" : "medium",
    });
    const table: [InjectPreflight, string][] = [
      [
        checked("elevated_target", "regedit.exe"),
        "目标窗口 regedit.exe 以管理员身份运行 · 文本只能复制到剪贴板",
      ],
      [checked("secure_desktop", null), "当前是安全桌面（UAC / 锁屏）· 无法插入文本"],
      [checked("proceed", "notepad.exe"), "目标窗口 notepad.exe · 可以插入"],
      [checked("unknown", null), "目标窗口未知 · 将直接尝试插入"],
    ];
    for (const [preflight, text] of table) {
      const view = renderApp({ path: "/onboarding?step=4", mock: { injectPreflight: preflight } });
      const line = await screen.findByTestId("trial-preflight");
      expect(line).toHaveTextContent(text);
      expect(line).toHaveAttribute("data-decision", preflight.decision);
      view.unmount();
    }
    // Default mock: an unchecked `proceed` (macOS / Linux have no UIPI) draws no line.
    const unchecked = renderApp({ path: "/onboarding?step=4" });
    await screen.findByRole("button", { name: "试说一句" });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId("trial-preflight")).toBeNull();
    unchecked.unmount();
    // A failing query draws nothing either (the injector reports the real outcome anyway).
    const failing = new MockBackend({ now: () => 1_758_700_000_000 });
    failing.injectPreflight = () => Promise.reject(new Error("no foreground window"));
    renderApp({ path: "/onboarding?step=4", backend: failing });
    await screen.findByRole("button", { name: "试说一句" });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId("trial-preflight")).toBeNull();
  });

  it("regression: the footer's Enter / Shift Enter / Esc are real; Enter follows the step's gate and fields keep their keys", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/onboarding",
      mock: { identity: { ...desktopIdentity(), platform: "linux", name: "ThinkPad" } },
    });
    await screen.findByTestId("nothing-to-grant");
    (document.activeElement as HTMLElement | null)?.blur();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "快捷键", level: 2 })).toBeInTheDocument();
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(await screen.findByTestId("nothing-to-grant")).toBeInTheDocument();
    await user.keyboard("{Enter}{Enter}");
    expect(
      await screen.findByRole("heading", { name: "选择语音模型", level: 2 }),
    ).toBeInTheDocument();
    // Step 3: Enter is 保存并继续 only while the choice is complete, and never from inside a field.
    const group = screen.getByRole("radiogroup", { name: "识别服务" });
    await user.click(within(group).getByRole("radio", { name: /其他服务商/ }));
    const key = screen.getByLabelText(/API 密钥/);
    await user.click(key);
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "选择语音模型", level: 2 })).toBeInTheDocument();
    key.blur();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "选择语音模型", level: 2 })).toBeInTheDocument();
    await user.click(within(group).getByRole("radio", { name: /内置服务/ }));
    (document.activeElement as HTMLElement | null)?.blur();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "试说一句", level: 2 })).toBeInTheDocument();
    expect(backend.peek().settings.engines.asr_provider).toBe("builtin");
    // Esc is 稍后设置.
    await user.keyboard("{Escape}");
    expect(await screen.findByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
  });

  it("regression: Enter does not pass a permission gate that 继续 would not", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ identity: mac() });
    backend.permissionsStatus = () =>
      Promise.resolve(macReport({ microphone: "denied", accessibility: "denied" }));
    renderApp({ path: "/onboarding", backend });
    expect(await screen.findByTestId("permission-hint")).toBeInTheDocument();
    (document.activeElement as HTMLElement | null)?.blur();
    await user.keyboard("{Enter}");
    expect(screen.getByTestId("permission-hint")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "快捷键", level: 2 })).toBeNull();
  });

  it("稍后设置 leaves the wizard from any step", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/onboarding?step=3" });
    await user.click(await screen.findByRole("button", { name: "稍后设置" }));
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
  });
});

describe("onboarding constants", () => {
  it("regression: the TCC reset names the bundle identifier tauri.conf.json ships", () => {
    const conf: unknown = JSON.parse(
      // vitest runs from apps/desktop (jsdom has no file: import.meta.url).
      readFileSync(join(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );
    const identifier =
      typeof conf === "object" && conf !== null && "identifier" in conf
        ? conf.identifier
        : undefined;
    expect(MACOS_BUNDLE_ID).toBe(identifier);
    expect(MACOS_TCC_RESET).toBe(`tccutil reset Accessibility ${MACOS_BUNDLE_ID}`);
    expect(MACOS_TCC_RESET_MICROPHONE).toBe(`tccutil reset Microphone ${MACOS_BUNDLE_ID}`);
  });
});
