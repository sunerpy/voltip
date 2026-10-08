import {
  type ProviderId,
  type ServiceKind,
  defaultEngineSettings,
  engineReady,
  providerStatus,
} from "@voltip/shared";
import {
  MOCK_ENGINE_BUILTIN,
  MOCK_MODEL_FILE,
  MOCK_MODEL_TICK_MS,
  MOCK_MODEL_TICKS,
  MOCK_MODELS_ROOT,
  MOCK_PROBE_MS,
  MOCK_GPU_HARDWARE,
  MOCK_STREAMING_MODEL_ID,
  MockBackend,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../../test/render";
import {
  activateLocalModel,
  applyProviderDraft,
  checkProviderDraft,
  downloadFraction,
  formatBytes,
  languageSummary,
  livePreviewState,
  modelAction,
  modelChoices,
  modelDescription,
  modelEngineLabel,
  modelStateCell,
  offlineModelsByTier,
  probeText,
  providerDraft,
  providersFor,
  serviceTarget,
  streamingModel,
  withProvider,
} from "./helpers";
import { gpuLabel, threadChoices } from "./LocalCompute";

const FIXED_WIDTH = /(^|\s)w-\[\d+px\]/;
const FIXED_HEIGHT = /(^|\s)h-\[\d+px\]/;
const PX_GRID_COLUMN = /grid-cols-\[[^\]]*\d+px/;
const SENSE_VOICE_BYTES = 239_549_735;

type User = ReturnType<typeof userEvent.setup>;

/** The page the model panes live on (语音模型 or AI 模型, pages of the main layout since
 *  2026-09-28). */
function modelsPage() {
  return screen.getByRole("main");
}

function modelCard(name: string) {
  return within(screen.getByTestId("local-models")).getByRole("article", { name });
}

function providerCard(kind: ServiceKind, id: ProviderId) {
  return screen.getByTestId(`provider-${kind}-${id}`);
}

/** The header toggle of a provider card (its accessible name starts with the provider's name). */
function cardToggle(kind: ServiceKind, id: ProviderId) {
  const card = providerCard(kind, id);
  const toggle = card.querySelector("button[aria-expanded]");
  if (!(toggle instanceof HTMLElement)) throw new Error(`no toggle on ${id}`);
  return toggle;
}

/** The views these tests move between: the 语音模型 page's two tabs (by their names in either
 *  locale), and the AI 模型 page, reached from the sidebar like a user would. */
async function openTab(user: User, name: string) {
  if (name === "AI 模型" || name === "AI models") {
    await user.click(await screen.findByRole("button", { name: /^(AI 模型|AI models)$/ }));
    await screen.findByTestId("page-ai");
    return;
  }
  if (screen.queryByTestId("page-speech") === null) {
    await user.click(await screen.findByRole("button", { name: /^(语音模型|Speech models)$/ }));
    await screen.findByTestId("page-speech");
  }
  await user.click(await screen.findByRole("radio", { name }));
}

async function openCard(user: User, kind: ServiceKind, id: ProviderId) {
  await screen.findByTestId(`provider-${kind}-${id}`);
  const toggle = cardToggle(kind, id);
  if (toggle.getAttribute("aria-expanded") !== "true") await user.click(toggle);
}

async function openLocalCard(user: User) {
  await openCard(user, "asr", "local");
  await screen.findByTestId("local-models");
}

describe("Settings · 语音模型 / AI 模型（服务商卡片）", () => {
  it("regression: the local card's 运行设备 writes device, GPU and threads through settings_set_engines, and offers a GPU only when the build has one (docs/dictation.md section 10.6)", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ hardware: MOCK_GPU_HARDWARE });
    renderApp({ path: "/speech", backend });
    await openLocalCard(user);
    const compute = await screen.findByTestId("local-compute");
    expect(compute).toHaveAttribute("data-device", "auto");
    const device = within(compute).getByRole("radiogroup", { name: "设备" });
    expect(
      within(compute).getByText("自动：有 GPU 时用 GPU，GPU 不可用时退回 CPU。"),
    ).toBeInTheDocument();
    await user.click(within(device).getByRole("radio", { name: "GPU" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.local_device).toBe("gpu");
    });
    expect(backend.peek().settings.engines.local_gpu).toBe("Vulkan0");
    const which = within(compute).getByRole("combobox", { name: "使用的 GPU" });
    expect(
      within(which)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["NVIDIA L40S · 45 GB 显存", "Intel UHD Graphics 770 · 集成显卡"]);
    await user.selectOptions(which, "Vulkan1");
    await waitFor(() => {
      expect(backend.peek().settings.engines.local_gpu).toBe("Vulkan1");
    });
    const threads = within(compute).getByRole("combobox", { name: "CPU 线程数" });
    expect(within(compute).getAllByText("CPU 线程数")).toHaveLength(1);
    expect(within(compute).getAllByText("使用的 GPU")).toHaveLength(1);
    expect(
      within(threads)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["自动（由模型决定）", "1", "2", "4", "8", "16"]);
    expect(within(compute).getByText("这台电脑有 16 个逻辑处理器。")).toBeInTheDocument();
    await user.selectOptions(threads, "4");
    await waitFor(() => {
      expect(backend.peek().settings.engines.local_threads).toBe(4);
    });
    await user.selectOptions(threads, "auto");
    await waitFor(() => {
      expect(backend.peek().settings.engines.local_threads).toBeNull();
    });
    await user.click(within(device).getByRole("radio", { name: "CPU" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.local_device).toBe("cpu");
    });
    expect(within(compute).queryByRole("combobox", { name: "使用的 GPU" })).toBeNull();
  });

  it("regression: a CPU-only build says the models run on the CPU and never offers the GPU", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/speech" });
    await openLocalCard(user);
    const compute = await screen.findByTestId("local-compute");
    expect(
      within(compute).getByText(
        "未找到本地模型可用的 GPU（Windows 和 Linux 需要支持 Vulkan 的显卡驱动），模型将在 CPU 上运行。",
      ),
    ).toBeInTheDocument();
    expect(within(compute).getByRole("radio", { name: "GPU" })).toBeDisabled();
    expect(threadChoices(8)).toEqual([1, 2, 4, 8]);
    expect(threadChoices(6)).toEqual([1, 2, 4, 6]);
    expect(threadChoices(0)).toEqual([]);
    // Regression: the core refuses more than MAX_LOCAL_THREADS (256); a bigger machine is offered
    // no count the save would reject.
    expect(threadChoices(256)).toEqual([1, 2, 4, 8, 16, 32, 64, 128, 256]);
    expect(threadChoices(384)).toEqual([1, 2, 4, 8, 16, 32, 64, 128, 256]);
    expect(
      gpuLabel(
        { name: "Metal", description: "", kind: "metal", memory_mb: 0, integrated: false },
        (k) => k,
      ),
    ).toBe("Metal · metal");
  });

  it("regression: /engines and the old settings groups open the 语音模型 page, no dialog", async () => {
    for (const path of ["/engines", "/settings/speech", "/settings/engine"]) {
      const { unmount } = renderApp({ path });
      const page = await screen.findByTestId("page-speech");
      expect(within(page).getByRole("heading", { name: "语音模型", level: 2 })).toBeInTheDocument();
      expect(within(page).getByTestId("speech-pane")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "语音模型", level: 1 })).toBeInTheDocument();
      expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
      expect(screen.queryByTestId("page-background")).toBeNull();
      unmount();
    }
  });

  it("regression: the old /settings/refine and /settings/ai deep links land on the AI 模型 page", async () => {
    for (const path of ["/settings/refine", "/settings/ai", "/ai"]) {
      const { unmount } = renderApp({ path });
      const page = await screen.findByTestId("page-ai");
      expect(within(page).getByTestId("ai-pane")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "AI 模型", level: 1 })).toBeInTheDocument();
      expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
      unmount();
    }
  });

  it("lists the recognition providers as cards in catalogue order, the one in use ringed, open and marked 使用中", async () => {
    renderApp({ path: "/speech" });
    const list = await screen.findByRole("list", { name: "语音识别服务商" });
    const cards = within(list).getAllByRole("article");
    expect(cards.map((c) => c.getAttribute("aria-label"))).toEqual([
      "内置服务",
      "本机",
      "OpenAI",
      "Groq",
      "硅基流动",
      "阿里云百炼",
      "自定义接口",
    ]);
    const builtin = providerCard("asr", "builtin");
    expect(builtin).toHaveAttribute("data-selected", "true");
    expect(builtin).toHaveAttribute("data-open", "true");
    expect(within(builtin).getByText("使用中")).toBeInTheDocument();
    expect(within(builtin).getByText("就绪")).toBeInTheDocument();
    expect(within(builtin).queryByRole("button", { name: "使用" })).toBeNull();
    expect(within(builtin).getByText(/内置服务的地址和密钥已包含在应用中/)).toBeInTheDocument();
    // Vendors without a key say so; nothing but the built-in card is open.
    const groq = providerCard("asr", "groq");
    expect(within(groq).getByText("缺少密钥")).toBeInTheDocument();
    expect(within(groq).getByText("whisper-large-v3-turbo")).toBeInTheDocument();
    expect(groq).not.toHaveAttribute("data-open");
    expect(within(providerCard("asr", "local")).getByText("本机运行")).toBeInTheDocument();
    expect(screen.getByTestId("current-asr")).toHaveTextContent("当前：内置服务 · Qwen3-ASR-1.7B");
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到内置服务");
    // No provider card for services a provider does not offer.
    expect(screen.queryByTestId("provider-asr-deepseek")).toBeNull();
    expect(screen.queryByTestId("provider-asr-ollama")).toBeNull();
    expect(modelsPage().textContent).not.toMatch(/计划中|尚未接入|示例/);
  });

  it("regression: the built-in service's host is never shown, in the pane, the title bar or the state", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await screen.findByTestId("providers-asr");
    await openTab(user, "AI 模型");
    await screen.findByTestId("providers-llm");
    const everything = [
      document.body.textContent ?? "",
      JSON.stringify(backend.peek().engines),
      [...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title")).join(" "),
    ].join(" ");
    expect(everything).not.toMatch(/voltip\.example|https?:\/\/[^ ]*builtin/);
    expect(backend.peek().engines.asr_host).toBe("");
    expect(backend.peek().engines.refine_host).toBe("");
    expect(providerStatus(backend.peek().engines, "builtin")?.asr?.base_url).toBeUndefined();
  });

  it("使用 switches the recognition provider; a vendor needs its key, which goes to provider_key_set and is never echoed", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await screen.findByTestId("provider-asr-groq");
    await user.click(within(providerCard("asr", "groq")).getByRole("button", { name: "使用" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual({
        ...defaultEngineSettings(),
        asr_provider: "groq",
      });
    });
    expect(await screen.findByText("已切换到Groq")).toBeInTheDocument();
    expect(engineReady(backend.peek().engines)).toBe(false);
    expect(screen.getByTestId("current-asr")).toHaveTextContent(
      "当前：Groq · whisper-large-v3-turbo",
    );
    const groq = providerCard("asr", "groq");
    expect(groq).toHaveAttribute("data-selected", "true");
    await openCard(user, "asr", "groq");
    const form = within(groq).getByTestId("provider-form");
    // The key field opens empty and stays a password field; saving without a key is refused.
    const key = within(form).getByLabelText("API 密钥");
    expect(key).toHaveValue("");
    expect(key).toHaveAttribute("type", "password");
    expect(key).toHaveAttribute("placeholder", "尚未设置");
    expect(within(form).getByText("Groq的语音识别与 AI 润色共用此密钥。")).toBeInTheDocument();
    await user.click(within(form).getByRole("button", { name: "保存" }));
    expect(within(form).getByTestId("provider-problem")).toHaveTextContent("请填写 API 密钥");
    await user.type(key, "gsk_secret_value_1234");
    await user.click(within(form).getByRole("button", { name: "显示密钥" }));
    expect(key).toHaveAttribute("type", "text");
    await user.click(within(form).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(engineReady(backend.peek().engines)).toBe(true);
    });
    expect(await screen.findByText(/已保存 · Groq · 密钥已写入系统钥匙串/)).toBeInTheDocument();
    expect(within(groq).getByText("就绪")).toBeInTheDocument();
    expect(within(form).getByTestId("provider-key-state")).toHaveTextContent("已设置");
    expect(key).toHaveValue("");
    expect(key).toHaveAttribute("placeholder", "已保存 · 留空则不改");
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到api.groq.com");
    // The key went only to provider_key_set; nothing in the UI or the state carries it.
    expect(JSON.stringify(backend.peek())).not.toContain("gsk_secret_value_1234");
    expect(document.body.textContent).not.toContain("gsk_secret_value_1234");
    // Deleting the key puts the card back to 缺少密钥.
    await user.click(within(form).getByRole("button", { name: "删除密钥" }));
    await waitFor(() => {
      expect(engineReady(backend.peek().engines)).toBe(false);
    });
    expect(within(groq).getByText("缺少密钥")).toBeInTheDocument();
    // 获取密钥 opens the vendor's page through the shell (the webview names the provider only).
    await user.click(within(form).getByRole("button", { name: "获取密钥" }));
    expect(backend.consolesOpened).toEqual(["groq"]);
  });

  it("the model select offers the presets, then what 测试连接 listed, then any other model id", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({
        path: "/speech",
        mock: { probeModels: { openai: ["whisper-2", "gpt-transcribe"] } },
      });
      await openCard(user, "asr", "openai");
      const form = within(providerCard("asr", "openai")).getByTestId("provider-form");
      const select = within(form).getByLabelText("模型");
      expect(
        within(select)
          .getAllByRole("option")
          .map((o) => o.textContent),
      ).toEqual(["gpt-transcribe", "gpt-4o-mini-transcribe", "whisper-1", "其他模型…"]);
      // No key yet: the probe is refused before any request.
      await user.click(within(form).getByRole("button", { name: "测试连接" }));
      await waitFor(() => {
        expect(within(form).getByTestId("probe-result")).toHaveTextContent("请先填写 API 密钥");
      });
      // With the key typed in the form (not saved): the provider's list joins the select.
      await user.type(within(form).getByLabelText("API 密钥"), "sk-draft");
      await user.click(within(form).getByRole("button", { name: "测试连接" }));
      act(() => {
        vi.advanceTimersByTime(MOCK_PROBE_MS);
      });
      await waitFor(() => {
        expect(within(form).getByTestId("probe-result")).toHaveTextContent(
          `连接正常 · 2 个模型 · ${MOCK_PROBE_MS} ms`,
        );
      });
      expect(
        within(select)
          .getAllByRole("option")
          .map((o) => o.textContent),
      ).toEqual([
        "gpt-transcribe",
        "gpt-4o-mini-transcribe",
        "whisper-1",
        "whisper-2",
        "其他模型…",
      ]);
      await user.selectOptions(select, "whisper-2");
      // 其他模型… opens a free field.
      await user.selectOptions(select, "__other__");
      const custom = within(form).getByLabelText("模型 ID");
      await user.type(custom, "my-asr");
      await user.click(within(form).getByRole("button", { name: "保存" }));
      await waitFor(() => {
        expect(backend.peek().settings.engines.providers?.openai?.asr_model).toBe("my-asr");
      });
      // The draft key was saved with it; nothing leaked.
      expect(JSON.stringify(backend.log)).not.toContain("sk-draft");
      // 恢复默认 clears the overrides (the key stays).
      await user.click(within(form).getByRole("button", { name: "恢复默认" }));
      await waitFor(() => {
        expect(backend.peek().settings.engines.providers).toBeUndefined();
      });
      expect(providerStatus(backend.peek().engines, "openai")?.asr?.key.set).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a custom endpoint needs an http(s) address and takes an optional key; the built-in key never follows it", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openCard(user, "asr", "custom");
    const card = providerCard("asr", "custom");
    expect(within(card).getByText("缺少接口地址")).toBeInTheDocument();
    const form = within(card).getByTestId("provider-form");
    expect(within(form).getByLabelText("API 密钥（可选）")).toBeInTheDocument();
    expect(within(form).queryByRole("button", { name: "获取密钥" })).toBeNull();
    // No presets: the model is a free field right away.
    const model = within(form).getByLabelText("模型 ID");
    await user.type(model, "whisper-large-v3");
    await user.click(within(form).getByRole("button", { name: "保存" }));
    expect(within(form).getByTestId("provider-problem")).toHaveTextContent("请填写接口地址");
    const url = within(form).getByLabelText("接口地址");
    await user.type(url, "asr.corp.local");
    await user.click(within(form).getByRole("button", { name: "保存" }));
    expect(within(form).getByTestId("provider-problem")).toHaveTextContent(
      "接口地址须以 http:// 或 https:// 开头",
    );
    await user.clear(url);
    await user.type(url, "http://192.168.1.20:8000/v1");
    await user.click(within(form).getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom).toEqual({
        asr_model: "whisper-large-v3",
        asr_url: "http://192.168.1.20:8000/v1",
      });
    });
    expect(within(card).getByText("就绪")).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "使用" }));
    await waitFor(() => {
      expect(backend.peek().engines.asr_provider).toBe("custom");
    });
    expect(backend.peek().engines.asr_host).toBe("192.168.1.20");
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到192.168.1.20");
    expect(providerStatus(backend.peek().engines, "custom")?.asr?.key).toEqual({
      set: false,
      source: "none",
    });
  });

  it("文本润色 lists the polish providers with the refine switch; switching provider and toggling write settings_set_engines", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openTab(user, "AI 模型");
    const list = await screen.findByRole("list", { name: "AI 润色服务商" });
    expect(
      within(list)
        .getAllByRole("article")
        .map((c) => c.getAttribute("aria-label")),
    ).toEqual([
      "内置服务",
      "OpenAI",
      "Groq",
      "Google AI Studio",
      "硅基流动",
      "阿里云百炼",
      "DeepSeek",
      "Ollama",
      "自定义接口",
    ]);
    expect(within(providerCard("llm", "builtin")).getByText("使用中")).toBeInTheDocument();
    expect(screen.getByTestId("current-llm")).toHaveTextContent("当前：内置服务 · qwen3.8-27b");
    expect(screen.getByTestId("privacy-llm")).toHaveTextContent("文本发送到内置服务");
    // Ollama: no key at all, but a model must be chosen.
    await openCard(user, "llm", "ollama");
    const ollama = providerCard("llm", "ollama");
    expect(within(ollama).getByText("未选择模型")).toBeInTheDocument();
    expect(within(ollama).queryByLabelText(/API 密钥/)).toBeNull();
    const toggle = screen.getByRole("switch", { name: /已开启/ });
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(screen.getByRole("switch", { name: /已关闭/ })).not.toBeChecked();
    await user.click(within(providerCard("llm", "deepseek")).getByRole("button", { name: "使用" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.llm_provider).toBe("deepseek");
    });
    expect(backend.peek().engines.refine_issue).toBe("key_missing");
    // The title bar's AI 润色 switch follows the same core state.
    expect(screen.getByTestId("polish-toggle")).toHaveAttribute("aria-pressed", "false");
  });

  it("regression: the insert setting lives only in Settings › Dictation, not among the recognition options", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/speech" });
    await openTab(user, "识别设置");
    await screen.findByLabelText("识别语言");
    expect(screen.queryByText("插入方式")).toBeNull();
    expect(screen.queryByRole("radio", { name: "仅复制到剪贴板" })).toBeNull();
    expect(screen.queryByRole("radio", { name: "粘贴到光标处" })).toBeNull();
  });

  it("识别设置 writes the language with the rest of the block unchanged", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openTab(user, "识别设置");
    // The row names the select once: no second 识别语言 caption above it (screenshots 2026-09-28).
    await screen.findByLabelText("识别语言");
    expect(screen.getAllByText("识别语言")).toHaveLength(1);
    await user.selectOptions(screen.getByLabelText("识别语言"), "zh");
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual({
        ...defaultEngineSettings(),
        language: "zh",
      });
    });
    await user.selectOptions(screen.getByLabelText("识别语言"), "");
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual(defaultEngineSettings());
    });
  });

  it("regression: on-device readiness follows the library — the card and the title bar agree", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/speech",
      backend: new MockBackend({
        settings: {
          engines: {
            ...defaultEngineSettings(),
            asr_provider: "local",
            local_model: "paraformer-zh",
          },
        },
      }),
    });
    const card = await screen.findByTestId("provider-asr-local");
    expect(engineReady(backend.peek().engines)).toBe(false);
    expect(within(card).getByText("模型未下载")).toBeInTheDocument();
    expect(card).toHaveAttribute("data-open", "true");
    await screen.findByTestId("local-models");
    const readout = screen.getByTestId("title-bar-readout");
    expect(within(readout).getByText("轻量 · 中文")).toBeInTheDocument();
    expect(within(readout).getByTestId("title-bar-readout-badge")).toHaveTextContent("本机");
    expect(within(readout).getByTitle("本机 · 轻量 · 中文 · 模型未下载")).toBeInTheDocument();
    expect(readout.querySelector("[data-tone='danger']")).not.toBeNull();
    act(() => {
      backend.publish({
        type: "engines",
        ...backend.peek().engines,
        local_ready: true,
        asr_ready: true,
        asr_issue: undefined,
      });
    });
    await waitFor(() => {
      expect(within(readout).getByTitle("本机 · 轻量 · 中文 · 就绪")).toBeInTheDocument();
    });
    expect(readout.querySelector("[data-tone='ok']")).not.toBeNull();
    await openTab(user, "识别设置");
    expect(screen.getByTestId("vad-trim")).toHaveAttribute("data-state", "off");
  });

  it("regression: activating a local model is settings_set_engines with the on-device provider and that model", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/speech",
      backend: new MockBackend({
        models: {
          "qwen3-asr-0.6b": {
            kind: "installed",
            path: `${MOCK_MODELS_ROOT}/qwen3-asr-0.6b`,
            installed_at: 1_790_100_000,
          },
        },
      }),
    });
    await openLocalCard(user);
    const balanced = modelCard("均衡");
    await user.click(within(balanced).getByRole("button", { name: "使用此模型" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual({
        ...defaultEngineSettings(),
        asr_provider: "local",
        local_model: "qwen3-asr-0.6b",
      });
    });
    expect(backend.peek().engines).toMatchObject({
      asr_provider: "local",
      local_model: "qwen3-asr-0.6b",
      local_ready: true,
      asr_host: "",
    });
    expect(await screen.findByText("已切换 · 本地识别 · 均衡")).toBeInTheDocument();
    expect(within(providerCard("asr", "local")).getByText("使用中")).toBeInTheDocument();
    expect(within(balanced).getByRole("button", { name: "当前使用" })).toBeDisabled();
    expect(within(balanced).getByTestId("model-installed")).toHaveTextContent(/已安装 · 2026/);
  });

  it("regression: download → progress → verifying → installed, and failure → retry, flow through MockBackend into the cards", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({ path: "/speech" });
      await openLocalCard(user);
      const sense = modelCard("轻量");
      await user.click(within(sense).getByRole("button", { name: "下载" }));
      // The first progress event lands at once: the download row with 0 bytes, a cancel button.
      await waitFor(() => {
        expect(within(sense).getByTestId("download-row")).toBeInTheDocument();
      });
      expect(within(sense).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "0");
      expect(within(sense).getByTestId("download-row")).toHaveTextContent(
        `0.0 KB / 239.5 MB · ${MOCK_MODEL_FILE}`,
      );
      expect(within(sense).getByText("下载中 0%")).toBeInTheDocument();
      expect(within(sense).getByRole("button", { name: "取消" })).toBeInTheDocument();
      // Half way: received / total in the row and the percentage on the badge.
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS / 2));
      });
      await waitFor(() => {
        expect(within(sense).getByText("下载中 50%")).toBeInTheDocument();
      });
      expect(within(sense).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
      expect(within(sense).getByTestId("download-row")).toHaveTextContent("119.8 MB / 239.5 MB");
      // The rest of the ticks, then the sha256 check, then installed with path and date.
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS / 2 + 1));
      });
      await waitFor(() => {
        expect(within(sense).getByTestId("verifying-row")).toBeInTheDocument();
      });
      expect(within(sense).getByText("校验中")).toBeInTheDocument();
      expect(within(sense).getByText("正在校验文件完整性…")).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS);
      });
      await waitFor(() => {
        expect(within(sense).getByText("已安装")).toBeInTheDocument();
      });
      expect(within(sense).getByTestId("model-installed")).toHaveTextContent(
        `${MOCK_MODELS_ROOT}/sense-voice-small`,
      );
      expect(within(sense).getByRole("button", { name: "使用此模型" })).toBeEnabled();
      expect(within(sense).getByRole("button", { name: "删除" })).toBeInTheDocument();
      expect(
        within(screen.getByTestId("local-models")).getByText("1 / 4 已安装"),
      ).toBeInTheDocument();
      expect(backend.peek().models[2]?.state.kind).toBe("installed");

      // Failure on the other model: the message is shown, 重试 downloads again.
      const para = modelCard("轻量 · 中文");
      await user.click(within(para).getByRole("button", { name: "下载" }));
      await waitFor(() => {
        expect(within(para).getByTestId("download-row")).toBeInTheDocument();
      });
      act(() => {
        backend.simulateModelFailed("paraformer-zh", "sha256 mismatch · model.int8.onnx");
      });
      await waitFor(() => {
        expect(within(para).getByTestId("model-failure")).toHaveTextContent(
          "失败原因 · sha256 mismatch · model.int8.onnx",
        );
      });
      expect(within(para).getByText("下载失败")).toBeInTheDocument();
      expect(within(para).queryByTestId("download-row")).toBeNull();
      await user.click(within(para).getByRole("button", { name: "重试" }));
      await waitFor(() => {
        expect(within(para).getByTestId("download-row")).toBeInTheDocument();
      });
      expect(within(para).queryByTestId("model-failure")).toBeNull();
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS * (MOCK_MODEL_TICKS + 3));
      });
      await waitFor(() => {
        expect(within(para).getByText("已安装")).toBeInTheDocument();
      });
      expect(
        within(screen.getByTestId("local-models")).getByText("2 / 4 已安装"),
      ).toBeInTheDocument();

      // Delete asks first, then model_remove: back to 未安装.
      await user.click(within(para).getByRole("button", { name: "删除" }));
      const confirm = screen.getByRole("dialog", { name: "删除模型 轻量 · 中文？" });
      expect(confirm).toHaveTextContent(`${MOCK_MODELS_ROOT}/paraformer-zh`);
      expect(confirm).toHaveTextContent("227.4 MB");
      await user.click(within(confirm).getByRole("button", { name: "取消" }));
      expect(backend.peek().models[3]?.state.kind).toBe("installed");
      await user.click(within(para).getByRole("button", { name: "删除" }));
      await user.click(
        within(screen.getByRole("dialog", { name: "删除模型 轻量 · 中文？" })).getByRole("button", {
          name: "删除",
        }),
      );
      await waitFor(() => {
        expect(backend.peek().models[3]?.state).toEqual({ kind: "not_installed" });
      });
      expect(await screen.findByText("已删除 · 轻量 · 中文")).toBeInTheDocument();
      expect(within(para).getByRole("button", { name: "下载" })).toBeInTheDocument();
      // Nothing in the flow was faked on the UI side: every state came from the backend's events.
      expect(backend.log.filter((e) => e.type === "models").length).toBeGreaterThan(
        MOCK_MODEL_TICKS * 2,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("手动下载 names the folder and every file's addresses, opens them through the shell or copies them, and an import names what it found missing (docs/dictation.md section 10)", async () => {
    // User request 2026-10-02: a network that cannot download from the app downloads in a browser.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    try {
      const { backend } = renderApp({ path: "/speech" });
      await openLocalCard(user);
      const sense = modelCard("轻量");
      const panel = within(sense).getByTestId("manual-download");
      const toggle = within(panel).getByRole("button", { name: "手动下载" });
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(within(panel).queryByTestId("manual-download-dir")).toBeNull();
      await user.click(toggle);
      const dir = `${MOCK_MODELS_ROOT}/sense-voice-small`;
      expect(within(panel).getByTestId("manual-download-dir")).toHaveTextContent(dir);
      const files = within(panel).getAllByTestId("manual-download-file");
      expect(files).toHaveLength(2);
      expect(files[0]).toHaveTextContent("model.int8.onnx · 239.2 MB");
      expect(files[1]).toHaveTextContent("tokens.txt · 315.9 KB");
      const repo = "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
      const mirror = `https://hf-mirror.com/${repo}/resolve/main/tokens.txt`;
      expect(
        within(files[1] as HTMLElement).getByText(
          `https://huggingface.co/${repo}/resolve/main/tokens.txt`,
        ),
      ).toBeInTheDocument();
      await user.click(
        within(files[1] as HTMLElement).getByRole("button", { name: `在浏览器中打开：${mirror}` }),
      );
      expect(backend.modelLinksOpened).toEqual([mirror]);
      // Another device may be the one that can reach the sources: each address copies too.
      await user.click(
        within(files[1] as HTMLElement).getByRole("button", { name: `复制链接：${mirror}` }),
      );
      expect(writeText).toHaveBeenCalledWith(mirror);
      expect(await screen.findByText("已复制链接")).toBeInTheDocument();
      await user.click(within(panel).getByRole("button", { name: "打开文件夹" }));
      expect(backend.modelFoldersOpened).toEqual(["sense-voice-small"]);
      await user.click(within(panel).getByRole("button", { name: "复制路径" }));
      expect(writeText).toHaveBeenCalledWith(dir);
      expect(await screen.findByText("已复制路径")).toBeInTheDocument();
      // An import that finds the folder incomplete names the files; the panel stays open.
      backend.simulateImportProblems("sense-voice-small", {
        missing: ["tokens.txt"],
        mismatched: ["model.int8.onnx"],
      });
      await user.click(within(panel).getByRole("button", { name: "检查并导入" }));
      await waitFor(() => {
        expect(within(sense).getByText("校验中")).toBeInTheDocument();
      });
      expect(within(sense).getByRole("button", { name: "检查并导入" })).toBeDisabled();
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS);
      });
      await waitFor(() => {
        expect(within(sense).getByText("导入未完成")).toBeInTheDocument();
      });
      const problems = within(sense).getByTestId("manual-download-problems");
      expect(problems).toHaveTextContent("缺少文件：tokens.txt");
      expect(problems).toHaveTextContent("校验不通过：model.int8.onnx");
      expect(within(sense).getByRole("button", { name: "下载" })).toBeEnabled();
      // The person put the right files there: the next import installs, and the panel goes.
      backend.simulateImportProblems("sense-voice-small", undefined);
      await user.click(within(sense).getByRole("button", { name: "检查并导入" }));
      act(() => {
        vi.advanceTimersByTime(MOCK_MODEL_TICK_MS);
      });
      await waitFor(() => {
        expect(within(sense).getByText("已安装")).toBeInTheDocument();
      });
      expect(within(sense).queryByTestId("manual-download")).toBeNull();
    } finally {
      vi.useRealTimers();
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  it("regression: the library lists recognition models by tier (均衡 → 高精度 → 轻量 → 轻量 · 中文) and the streaming model only in the 实时预览 block, without 使用此模型", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/speech",
      backend: new MockBackend({
        models: {
          [MOCK_STREAMING_MODEL_ID]: {
            kind: "installed",
            path: `${MOCK_MODELS_ROOT}/${MOCK_STREAMING_MODEL_ID}`,
            installed_at: 1_790_100_000,
          },
        },
      }),
    });
    await openLocalCard(user);
    const library = await screen.findByRole("list", { name: "本地模型库" });
    const items = within(library).getAllByRole("listitem");
    expect(items.map((li) => li.getAttribute("data-tier"))).toEqual([
      "balanced",
      "accurate",
      "light",
      "light",
    ]);
    expect(items.map((li) => within(li).getByRole("article").getAttribute("aria-label"))).toEqual([
      "均衡",
      "高精度",
      "轻量",
      "轻量 · 中文",
    ]);
    expect(within(library).queryByRole("article", { name: "实时预览" })).toBeNull();
    expect(
      within(modelCard("高精度")).getByText("qwen3-asr-1.7b · transcribe.cpp"),
    ).toBeInTheDocument();
    expect(within(modelCard("高精度")).getAllByText("1692.6 MB").length).toBeGreaterThan(0);
    // The 实时预览 block (识别设置): eyebrow, switch on, ready (the model is installed), one card.
    await openTab(user, "识别设置");
    const live = screen.getByTestId("live-preview");
    expect(live).toHaveAttribute("data-state", "ready");
    expect(within(live).getByText("实时预览", { selector: ".eyebrow" })).toBeInTheDocument();
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("已就绪");
    expect(within(live).getByRole("switch", { name: "实时预览" })).toBeChecked();
    const card = within(live).getByRole("article", { name: "实时预览" });
    expect(
      within(card).getByText(`${MOCK_STREAMING_MODEL_ID} · Zipformer 流式`),
    ).toBeInTheDocument();
    expect(within(card).getByText("已安装")).toBeInTheDocument();
    expect(within(card).getByTestId("model-installed")).toHaveTextContent(
      `${MOCK_MODELS_ROOT}/${MOCK_STREAMING_MODEL_ID}`,
    );
    // Not a recognition model: no 使用此模型 / 当前使用, only 删除 (and no 推荐 badge).
    expect(within(card).queryByRole("button", { name: "使用此模型" })).toBeNull();
    expect(within(card).queryByText("当前使用")).toBeNull();
    expect(within(card).queryByText("推荐")).toBeNull();
    expect(within(card).getByRole("button", { name: "删除" })).toBeInTheDocument();
    expect(backend.peek().models.find((m) => m.id === MOCK_STREAMING_MODEL_ID)?.active).toBe(false);
    // Delete → model_remove; the block says the model is missing and offers 下载 again.
    await user.click(within(card).getByRole("button", { name: "删除" }));
    await user.click(
      within(screen.getByRole("dialog", { name: "删除模型 实时预览？" })).getByRole("button", {
        name: "删除",
      }),
    );
    await waitFor(() => {
      expect(backend.peek().engines.live_preview_ready).toBe(false);
    });
    expect(live).toHaveAttribute("data-state", "missing");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("模型未下载");
    expect(within(card).getByRole("button", { name: "下载" })).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "使用此模型" })).toBeNull();
    // The engines pane is still fluid: the block uses the same auto-fill grid.
    expect(within(live).getByRole("list", { name: "实时预览" }).style.gridTemplateColumns).toBe(
      "repeat(auto-fill, minmax(300px, 1fr))",
    );
  });

  it("with the built-in service the 实时预览 block is ready without the model, and the streaming output modes take effect (docs/dictation.md section 11.8)", async () => {
    // M8 2026-10-02 (user request 2026-09-30: Qwen3-ASR previews itself): a release build's
    // built-in service previews while recording; no model needs downloading.
    const user = userEvent.setup();
    renderApp({
      path: "/speech",
      mock: {
        builtIn: {
          asr: { model: MOCK_ENGINE_BUILTIN.asr_model, key: true, preview: true },
          llm: { model: MOCK_ENGINE_BUILTIN.refine_model, key: true },
        },
      },
    });
    await openTab(user, "识别设置");
    const live = await screen.findByTestId("live-preview");
    expect(live).toHaveAttribute("data-state", "cloud");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("已就绪 · 内置服务");
    expect(live).toHaveTextContent("使用内置服务时，预览由内置服务提供");
  });

  it("regression: on Model Studio's realtime model the 实时预览 block is ready without the model, and 整段输出 says it runs as 边说边识别 (docs/dictation.md section 11.9)", async () => {
    // Goal 2026-10-03: qwen-audio-3.1-asr-flash-streaming failed; it streams the take itself now.
    const user = userEvent.setup();
    renderApp({
      path: "/speech",
      mock: {
        settings: { engines: { ...defaultEngineSettings(), asr_provider: "aliyun" } },
        providerKeys: [{ provider: "aliyun", kind: "asr" }],
      },
    });
    await openTab(user, "识别设置");
    const live = await screen.findByTestId("live-preview");
    expect(live).toHaveAttribute("data-state", "stream");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent(
      "已就绪 · 实时识别模型",
    );
    const mode = screen.getByTestId("output-mode");
    expect(mode).toHaveAttribute("data-mode", "whole_take");
    expect(mode).toHaveAttribute("data-effective", "streaming_final");
    expect(within(mode).getByTestId("output-mode-streamed")).toHaveTextContent(
      "所选识别模型是实时识别模型",
    );
    expect(within(mode).getByTestId("output-mode-state")).toHaveTextContent("边说边识别");
    expect(screen.queryByTestId("output-mode-fallback")).toBeNull();
  });

  it("a whole-file Model Studio model keeps the old preview rules and the custom endpoint takes a Model Studio address", async () => {
    const user = userEvent.setup();
    renderApp({
      path: "/speech",
      mock: {
        settings: {
          engines: {
            ...defaultEngineSettings(),
            asr_provider: "custom",
            providers: {
              custom: {
                asr_url: "https://ws-test.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                asr_model: "qwen-audio-3.1-asr-flash",
              },
            },
          },
        },
      },
    });
    await openTab(user, "识别设置");
    const live = await screen.findByTestId("live-preview");
    expect(live).toHaveAttribute("data-state", "missing");
    expect(screen.getByTestId("output-mode")).toHaveAttribute("data-effective", "whole_take");
    expect(screen.queryByTestId("output-mode-streamed")).toBeNull();
  });

  it("regression: the 实时预览 toggle writes live_preview through settings_set_engines and the state line follows live_preview_ready", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openTab(user, "识别设置");
    const live = await screen.findByTestId("live-preview");
    // Default: on, but the streaming model is not downloaded.
    expect(backend.peek().settings.engines.live_preview).toBe(true);
    expect(live).toHaveAttribute("data-state", "missing");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("模型未下载");
    const toggle = within(live).getByRole("switch", { name: "实时预览" });
    expect(toggle).toBeChecked();
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.engines.live_preview).toBe(false);
    });
    // The whole engines block went out with only live_preview flipped.
    expect(backend.peek().settings.engines).toEqual({
      ...defaultEngineSettings(),
      live_preview: false,
    });
    expect(live).toHaveAttribute("data-state", "off");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("已关闭");
    expect(within(live).getByRole("switch", { name: "实时预览" })).not.toBeChecked();
    // Installing the model while off: still off. Switching back on: ready.
    act(() => {
      backend.publish({
        type: "models",
        models: backend.peek().models.map((m) =>
          m.id === MOCK_STREAMING_MODEL_ID
            ? {
                ...m,
                state: {
                  kind: "installed",
                  path: `${MOCK_MODELS_ROOT}/${MOCK_STREAMING_MODEL_ID}`,
                  installed_at: 1_758_700_000,
                },
              }
            : m,
        ),
      });
    });
    expect(live).toHaveAttribute("data-state", "off");
    await user.click(within(live).getByRole("switch", { name: "实时预览" }));
    await waitFor(() => {
      expect(backend.peek().engines.live_preview_ready).toBe(true);
    });
    expect(live).toHaveAttribute("data-state", "ready");
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("已就绪");
    expect(backend.peek().settings.engines).toEqual(defaultEngineSettings());
    expect(backend.log.filter((e) => e.type === "settings")).toHaveLength(2);
    // No words about the streaming path being a recognition engine anywhere on the page.
    expect(modelsPage().textContent).not.toMatch(/计划中|尚未接入|示例/);
  });

  it("regression: the output mode cards (出字方式) write output_mode through settings_set_engines; a streaming mode without the model stays selected but runs as 整段输出 (status line, card note, 当前生效 badge follow effective_output_mode); live_inject with polish on shows the no-polish note; 静音裁剪 is disabled under cloud and writes vad_trim under local (docs/dictation.md §12)", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openTab(user, "识别设置");
    await screen.findByTestId("output-mode");
    const block = () => screen.getByTestId("output-mode");
    expect(block()).toHaveAttribute("data-mode", "whole_take");
    expect(block()).toHaveAttribute("data-effective", "whole_take");
    const modes = () => within(block()).getByRole("listbox", { name: "输出方式" });
    const options = within(modes()).getAllByRole("option");
    expect(options.map((o) => o.getAttribute("aria-label"))).toEqual([
      "整段输出",
      "边说边识别",
      "边说边输入",
    ]);
    expect(within(modes()).getByRole("option", { name: "整段输出" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(block()).getByTestId("output-mode-state")).toHaveTextContent("整段输出");
    expect(within(block()).getAllByText("当前生效")).toHaveLength(1);
    expect(
      within(modes()).getByText("松开快捷键后一次性完成识别、润色和插入。"),
    ).toBeInTheDocument();
    expect(
      within(modes()).getByText(/此方式不进行润色，取消后已输入的文字不会撤回/),
    ).toBeInTheDocument();
    // No streaming model yet: both streaming cards carry the missing-model badge.
    expect(within(modes()).getAllByText("模型未下载")).toHaveLength(2);
    expect(screen.queryByTestId("output-mode-fallback")).toBeNull();
    expect(screen.queryByTestId("refine-live-inject")).toBeNull();
    // Pick 实时注入: the whole engines block goes out with output_mode flipped; the model is
    // missing, so the card is selected but the take runs as a whole take, and says so.
    await user.click(within(modes()).getByRole("option", { name: "边说边输入" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines).toEqual({
        ...defaultEngineSettings(),
        output_mode: "live_inject",
      });
    });
    expect(backend.peek().engines.effective_output_mode).toBe("whole_take");
    expect(block()).toHaveAttribute("data-mode", "live_inject");
    expect(block()).toHaveAttribute("data-effective", "whole_take");
    expect(within(modes()).getByRole("option", { name: "边说边输入" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(within(block()).getByTestId("output-mode-state")).toHaveTextContent(
      "实时识别模型未下载，当前按整段输出运行",
    );
    expect(within(block()).getByTestId("output-mode-fallback")).toHaveTextContent(
      "实时识别模型未下载，当前按整段输出运行",
    );
    // The 当前生效 badge stays on 整段输出 (what really runs).
    expect(
      within(within(modes()).getByRole("option", { name: "整段输出" })).getByText("当前生效"),
    ).toBeInTheDocument();
    // Polish is on: the polish view explains live_inject never refines.
    await openTab(user, "AI 模型");
    expect(screen.getByTestId("refine-live-inject")).toHaveTextContent(
      "「边说边输入」模式下不进行润色",
    );
    await openTab(user, "识别设置");
    // Re-selecting the current card writes nothing.
    const writes = () => backend.log.filter((e) => e.type === "settings").length;
    const before = writes();
    await user.click(within(modes()).getByRole("option", { name: "边说边输入" }));
    expect(writes()).toBe(before);
    // The streaming model lands: the mode takes effect, the notes disappear, the badge moves.
    act(() => {
      backend.publish({
        type: "models",
        models: backend.peek().models.map((m) =>
          m.id === MOCK_STREAMING_MODEL_ID
            ? {
                ...m,
                state: {
                  kind: "installed",
                  path: `${MOCK_MODELS_ROOT}/${MOCK_STREAMING_MODEL_ID}`,
                  installed_at: 1_758_700_000,
                },
              }
            : m,
        ),
      });
      backend.publish({
        type: "engines",
        ...backend.peek().engines,
        live_preview_ready: true,
        effective_output_mode: "live_inject",
      });
    });
    await waitFor(() => {
      expect(block()).toHaveAttribute("data-effective", "live_inject");
    });
    expect(within(block()).getByTestId("output-mode-state")).toHaveTextContent("边说边输入");
    expect(screen.queryByTestId("output-mode-fallback")).toBeNull();
    expect(within(modes()).queryByText("模型未下载")).toBeNull();
    expect(
      within(within(modes()).getByRole("option", { name: "边说边输入" })).getByText("当前生效"),
    ).toBeInTheDocument();
    // Turning polish off removes the note; 流式定稿 keeps polish available (no note).
    await openTab(user, "AI 模型");
    await user.click(screen.getByRole("switch", { name: /已开启/ }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(screen.queryByTestId("refine-live-inject")).toBeNull();
    await openTab(user, "识别设置");
    await user.click(
      within(screen.getByRole("listbox", { name: "输出方式" })).getByRole("option", {
        name: "边说边识别",
      }),
    );
    await waitFor(() => {
      expect(backend.peek().settings.engines.output_mode).toBe("streaming_final");
    });
    expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    await openTab(user, "AI 模型");
    await user.click(screen.getByRole("switch", { name: /已关闭/ }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(true);
    });
    expect(screen.queryByTestId("refine-live-inject")).toBeNull();
    await openTab(user, "识别设置");

    // 静音裁剪: a remote provider → disabled with the reason; on-device → the switch writes vad_trim.
    let vad = screen.getByTestId("vad-trim");
    expect(vad).toHaveAttribute("data-state", "cloud");
    expect(within(vad).getByRole("switch", { name: "静音裁剪" })).toBeDisabled();
    expect(within(vad).getByTestId("vad-trim-cloud")).toHaveTextContent(
      "仅本地识别可用；使用其他服务商时不裁剪。",
    );
    expect(within(vad).getByText(/本地识别前用 Silero VAD 去掉首尾静音/)).toBeInTheDocument();
    await openTab(user, "服务商与模型");
    await user.click(within(providerCard("asr", "local")).getByRole("button", { name: "使用" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.asr_provider).toBe("local");
    });
    await openTab(user, "识别设置");
    vad = screen.getByTestId("vad-trim");
    expect(vad).toHaveAttribute("data-state", "off");
    expect(within(vad).queryByTestId("vad-trim-cloud")).toBeNull();
    const toggle = within(vad).getByRole("switch", { name: "静音裁剪" });
    expect(toggle).toBeEnabled();
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.engines.vad_trim).toBe(true);
    });
    expect(backend.peek().settings.engines).toMatchObject({
      asr_provider: "local",
      output_mode: "streaming_final",
      vad_trim: true,
      refine_enabled: true,
    });
    expect(vad).toHaveAttribute("data-state", "on");
    expect(within(vad).getByTestId("vad-trim-state")).toHaveTextContent("开");
    // Nothing on the pane pretends to be unwired or English-labelled.
    expect(modelsPage().textContent).not.toMatch(/尚未接入|示例|whole_take|live_inject/);
  });

  it("regression: the engines pane is fluid and scrolls inside the page — no fixed-width panels", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/speech" });
    await openLocalCard(user);
    const pane = screen.getByTestId("speech-pane");
    const isControl = (el: Element) =>
      el.closest('[role="switch"], [role="radiogroup"], button, svg, [role="progressbar"]') !==
      null;
    for (const el of [pane, ...pane.querySelectorAll("[class]")].filter(
      (node) => !isControl(node),
    )) {
      const cls = el.getAttribute("class") ?? "";
      expect(cls).not.toMatch(FIXED_WIDTH);
      expect(cls).not.toMatch(FIXED_HEIGHT);
      expect(cls).not.toMatch(PX_GRID_COLUMN);
    }
    expect(screen.getByRole("list", { name: "本地模型库" }).style.gridTemplateColumns).toBe(
      "repeat(auto-fill, minmax(260px, 1fr))",
    );
    const content = screen.getByRole("main");
    expect(content.className).toMatch(/min-h-0 flex-1 overflow-auto/);
    expect(content.contains(pane)).toBe(true);
  });

  // Regression (2026-09-27 screenshot): Qwen3-ASR's ten languages sat in one badge that does not
  // wrap, which pushed the 均衡 card over its neighbour in the grid.
  it("regression: a model card folds a long language list and keeps the listed languages in the tooltip", async () => {
    const ten = ["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"];
    expect(languageSummary(ten)).toEqual({
      text: "zh · en · ja · ko · yue 等",
      title: "语言：zh, en, ja, ko, yue, de, fr, es, ru, ar 等",
    });
    expect(languageSummary(["zh", "en"])).toEqual({ text: "zh · en", title: undefined });
    expect(languageSummary(ten.slice(0, 5)).text).toBe("zh · en · ja · ko · yue");
    const user = userEvent.setup();
    renderApp({ path: "/speech" });
    await openLocalCard(user);
    const library = await screen.findByRole("list", { name: "本地模型库" });
    const balanced = within(library)
      .getAllByRole("listitem")
      .find((li) => li.getAttribute("data-tier") === "balanced");
    expect(balanced).toBeDefined();
    const badge = within(balanced as HTMLElement).getByText("zh · en · ja · ko · yue 等");
    expect(badge).toHaveAttribute("title", "语言：zh, en, ja, ko, yue, de, fr, es, ru, ar 等");
  });

  it("the provider helpers and the model helpers are pure", () => {
    const base = defaultEngineSettings();
    const status = new MockBackend().peek().engines;
    expect(providersFor(status, "asr").map((p) => p.id)).toEqual([
      "builtin",
      "local",
      "openai",
      "groq",
      "siliconflow",
      "aliyun",
      "custom",
    ]);
    expect(providersFor(status, "llm").map((p) => p.id)).toContain("ollama");
    expect(providerDraft(base, "groq", "asr")).toEqual({ model: "", baseUrl: "", key: "" });
    const withGroq = applyProviderDraft(base, "groq", "llm", {
      model: " qwen/qwen3.8-27b ",
      baseUrl: "",
    });
    expect(withGroq).toEqual({ ...base, providers: { groq: { llm_model: "qwen/qwen3.8-27b" } } });
    expect(providerDraft(withGroq, "groq", "llm").model).toBe("qwen/qwen3.8-27b");
    expect(applyProviderDraft(withGroq, "groq", "llm", { model: "", baseUrl: "" })).toEqual(base);
    expect(withProvider(base, "asr", "local").asr_provider).toBe("local");
    expect(withProvider(base, "llm", "ollama").llm_provider).toBe("ollama");
    const groq = providerStatus(status, "groq");
    if (groq?.asr === undefined) throw new Error("groq card");
    expect(modelChoices(groq.asr, ["whisper-large-v3", "extra"])).toEqual([
      "whisper-large-v3-turbo",
      "whisper-large-v3",
      "extra",
    ]);
    expect(checkProviderDraft({ model: "", baseUrl: "", key: "" }, groq, "asr")).toBe(
      "请填写 API 密钥",
    );
    expect(checkProviderDraft({ model: "", baseUrl: "", key: "k" }, groq, "asr")).toBeUndefined();
    expect(checkProviderDraft({ model: "", baseUrl: "api", key: "k" }, groq, "asr")).toBe(
      "接口地址须以 http:// 或 https:// 开头",
    );
    const custom = providerStatus(status, "custom");
    if (custom === undefined) throw new Error("custom card");
    expect(checkProviderDraft({ model: "m", baseUrl: "", key: "" }, custom, "asr")).toBe(
      "请填写接口地址",
    );
    expect(checkProviderDraft({ model: "", baseUrl: "http://h", key: "" }, custom, "asr")).toBe(
      "请选择或填写模型",
    );
    expect(
      probeText({ provider: "groq", kind: "llm", result: "ok", models: ["a"], latency_ms: 9 }),
    ).toEqual({ ok: true, text: "连接正常 · 1 个模型 · 9 ms" });
    expect(
      probeText({
        provider: "groq",
        kind: "llm",
        result: "failed",
        reason: "http_status",
        status: 502,
      }).text,
    ).toBe("服务返回错误（HTTP 502）");
    expect(serviceTarget("builtin", "")).toBe("内置服务");
    expect(serviceTarget("local", "")).toBeUndefined();
    expect(serviceTarget("ollama", "127.0.0.1")).toBeUndefined();
    expect(serviceTarget("groq", "api.groq.com")).toBe("api.groq.com");
    expect(activateLocalModel(base, "paraformer-zh")).toEqual({
      ...base,
      asr_provider: "local",
      local_model: "paraformer-zh",
    });
    expect(formatBytes(SENSE_VOICE_BYTES)).toBe("239.5 MB");
    expect(formatBytes(315_894)).toBe("315.9 KB");
    expect(formatBytes(0)).toBe("0.0 KB");
    expect(downloadFraction({ kind: "not_installed" })).toBe(0);
    expect(downloadFraction({ kind: "downloading", received: 5, total: 0, file: "f" })).toBe(0);
    expect(downloadFraction({ kind: "downloading", received: 50, total: 200, file: "f" })).toBe(
      0.25,
    );
    expect(downloadFraction({ kind: "downloading", received: 300, total: 200, file: "f" })).toBe(1);
    expect(modelStateCell({ kind: "installed", path: "/p", installed_at: 1 })).toEqual({
      tone: "ok",
      text: "已安装",
    });
    expect(modelStateCell({ kind: "downloading", received: 1, total: 4, file: "f" })).toEqual({
      tone: "warn",
      text: "下载中 25%",
    });
    expect(modelStateCell({ kind: "verifying" })).toEqual({ tone: "accent", text: "校验中" });
    expect(modelStateCell({ kind: "not_installed" })).toEqual({ tone: "idle", text: "未安装" });
    expect(modelStateCell({ kind: "failed", message: "x" })).toEqual({
      tone: "danger",
      text: "下载失败",
    });
    const model = new MockBackend().peek().models[0];
    if (!model) throw new Error("catalogue");
    expect(modelAction(model)).toBe("download");
    expect(
      modelAction({ ...model, state: { kind: "downloading", received: 0, total: 1, file: "f" } }),
    ).toBe("cancel");
    expect(modelAction({ ...model, state: { kind: "verifying" } })).toBe("cancel");
    expect(modelAction({ ...model, state: { kind: "failed", message: "m" } })).toBe("retry");
    const installed = {
      ...model,
      state: { kind: "installed" as const, path: "/p", installed_at: 1 },
    };
    expect(modelAction(installed)).toBe("use");
    expect(modelAction({ ...installed, active: true })).toBe("current");
    // The streaming preview model is never a recognition choice: installed means done.
    const streaming = new MockBackend().peek().models.find((m) => m.id === MOCK_STREAMING_MODEL_ID);
    if (!streaming) throw new Error("catalogue");
    expect(modelAction(streaming)).toBe("download");
    expect(modelAction({ ...streaming, state: installed.state })).toBe("installed");
    expect(modelAction({ ...streaming, state: installed.state, active: true })).toBe("installed");
    expect(modelDescription(model)).toBe("推荐；Qwen3-ASR 0.6B，30 语种自动识别，自带标点；690 MB");
    expect(modelDescription({ ...model, id: "paraformer-zh" })).toBe(
      "Paraformer 中文（含方言）更准，中英混读；无标点，开启 AI 润色可补；227 MB",
    );
    expect(modelDescription({ ...model, id: "sense-voice-small" }, "en")).toMatch(
      /^SenseVoice Small/,
    );
    expect(modelDescription({ ...model, id: "future-model", description: "core text" })).toBe(
      "core text",
    );
    expect(modelEngineLabel("transcribe_cpp")).toBe("transcribe.cpp");
    expect(modelEngineLabel("sense_voice")).toBe("SenseVoice");
    expect(modelEngineLabel("paraformer")).toBe("Paraformer");
    expect(modelEngineLabel("zipformer_streaming")).toBe("Zipformer 流式");
    // Tier order for the library (a stable sort: the core's order within a tier), the streaming
    // model on its own.
    const models = new MockBackend().peek().models;
    const shuffled = [models[2], models[4], models[3], models[1], models[0]].flatMap((m) =>
      m ? [m] : [],
    );
    expect(offlineModelsByTier(shuffled).map((m) => m.id)).toEqual([
      "qwen3-asr-0.6b",
      "qwen3-asr-1.7b",
      "sense-voice-small",
      "paraformer-zh",
    ]);
    expect(streamingModel(shuffled)?.id).toBe(MOCK_STREAMING_MODEL_ID);
    expect(streamingModel([])).toBeUndefined();
    expect(offlineModelsByTier([])).toEqual([]);
    expect(livePreviewState({ live_preview: false }, { live_preview_ready: false })).toBe("off");
    expect(livePreviewState({ live_preview: false }, { live_preview_ready: true })).toBe("off");
    expect(livePreviewState({ live_preview: true }, { live_preview_ready: false })).toBe("missing");
    expect(livePreviewState({ live_preview: true }, { live_preview_ready: true })).toBe("ready");
    for (const source of ["cloud", "stream", "local"] as const) {
      expect(
        livePreviewState({ live_preview: true }, { live_preview_ready: true, live_source: source }),
      ).toBe(source === "local" ? "ready" : source);
    }
  });

  it("regression: the Chinese script choice writes chinese_script through settings_set_engines with the three single-language options", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/speech" });
    await openTab(user, "识别设置");
    const section = await screen.findByTestId("chinese-script");
    expect(section).toHaveAttribute("data-script", "simplified");
    expect(within(section).getByText("中文字形", { selector: ".eyebrow" })).toBeInTheDocument();
    const group = within(section).getByRole("radiogroup", { name: "中文字形" });
    expect(
      within(group)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["简体", "繁体", "保持原样"]);
    expect(within(group).getByRole("radio", { name: "简体" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(group).getByRole("radio", { name: "繁体" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.chinese_script).toBe("traditional");
    });
    // The rest of the engine block is sent unchanged.
    expect(backend.peek().settings.engines).toEqual({
      ...defaultEngineSettings(),
      chinese_script: "traditional",
    });
    expect(screen.getByTestId("chinese-script")).toHaveAttribute("data-script", "traditional");
    await user.click(within(group).getByRole("radio", { name: "保持原样" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.chinese_script).toBe("as_is");
    });
    // Picking the current script sends nothing.
    const log = backend.log.length;
    await user.click(within(group).getByRole("radio", { name: "保持原样" }));
    expect(backend.log).toHaveLength(log);
  });

  it("regression: the Chinese script options are single-language in English too", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/speech", mock: { settings: { locale: "en" } } });
    await openTab(user, "Recognition");
    const section = await screen.findByTestId("chinese-script");
    const group = within(section).getByRole("radiogroup", { name: "Chinese script" });
    expect(
      within(group)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["Simplified", "Traditional", "As recognised"]);
  });
});

describe("fallback models on the engines pages (docs/dictation.md §3.5)", () => {
  it("names the fallback model standing in, and where the audio goes once the quota runs out", async () => {
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_fallback: { enabled: true, models: [{ provider: "groq", model: "whisper-large-v3" }] },
      },
    });
    renderApp({ path: "/speech", backend });
    expect(await screen.findByTestId("fallback-asr")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByTestId("privacy-asr")).toHaveTextContent("额度用完时改发给Groq");
    });
    expect(screen.getByTestId("current-asr")).not.toHaveTextContent("候补");
    act(() => {
      backend.simulateQuotaExhausted("asr", Date.now() + 86_400_000);
    });
    await waitFor(() => {
      expect(screen.getByTestId("current-asr")).toHaveTextContent(
        "当前：Groq · whisper-large-v3（候补）",
      );
    });
  });

  it("regression: with the switch off, or a selected service the chain does not run for, the privacy line names nobody else", async () => {
    // Goal review 2026-10-04: the line named the fallback providers whatever the switch said.
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const state = await backend.getState();
    const listed = { provider: "groq" as const, model: "whisper-large-v3" };
    await backend.invoke("settings_set_engines", {
      engines: { ...state.settings.engines, asr_fallback: { enabled: false, models: [listed] } },
    });
    renderApp({ path: "/speech", backend });
    expect(await screen.findByTestId("fallback-asr")).toHaveAttribute("data-enabled", "false");
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到内置服务");
    expect(screen.getByTestId("privacy-asr")).not.toHaveTextContent("额度用完");
    // On, with the selected provider lacking its key: the chain does not run either.
    await act(async () => {
      await backend.invoke("settings_set_engines", {
        engines: {
          ...state.settings.engines,
          asr_provider: "openai",
          asr_fallback: { enabled: true, models: [listed] },
        },
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("fallback-asr-not-in-use")).toBeInTheDocument();
    });
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到OpenAI");
    expect(screen.getByTestId("privacy-asr")).not.toHaveTextContent("额度用完");
  });

  it("regression: a realtime model that ran out, with a whole-recording model standing in, pauses the preview and runs whole takes", async () => {
    // Goal review 2026-10-04: 边说边识别 stayed 当前生效 while the takes ran whole.
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "aliyun", kind: "asr" }] });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_provider: "aliyun",
        asr_fallback: {
          enabled: true,
          models: [{ provider: "aliyun", model: "qwen-audio-3.1-asr-flash" }],
        },
      },
    });
    renderApp({ path: "/speech", backend });
    await openTab(user, "识别设置");
    const live = await screen.findByTestId("live-preview");
    const mode = screen.getByTestId("output-mode");
    expect(live).toHaveAttribute("data-state", "stream");
    expect(mode).toHaveAttribute("data-effective", "streaming_final");
    act(() => {
      backend.simulateQuotaExhausted("asr", Date.now() + 86_400_000);
    });
    await waitFor(() => {
      expect(live).toHaveAttribute("data-state", "paused");
    });
    expect(within(live).getByTestId("live-preview-state")).toHaveTextContent(
      "暂不可用 · 候补模型不支持实时识别",
    );
    expect(mode).toHaveAttribute("data-effective", "whole_take");
    expect(within(mode).getByTestId("output-mode-state")).toHaveTextContent("整段输出");
    expect(screen.queryByTestId("output-mode-streamed")).toBeNull();
    expect(within(mode).queryByText("模型未下载")).toBeNull();
    // A streaming mode asked for runs whole too, and says why without asking for a download.
    await user.click(within(mode).getByRole("option", { name: "边说边识别" }));
    await waitFor(() => {
      expect(within(mode).getByTestId("output-mode-fallback")).toHaveTextContent(
        "当前使用的候补模型不支持实时识别，按整段输出运行",
      );
    });
    // 重新检查: the realtime model streams again.
    await act(async () => {
      await backend.invoke("engines_quota_reset", { kind: "asr" });
    });
    await waitFor(() => {
      expect(live).toHaveAttribute("data-state", "stream");
    });
    expect(mode).toHaveAttribute("data-effective", "streaming_final");
  });

  it("the AI page carries the clean-up's list", async () => {
    renderApp({ path: "/ai" });
    expect(await screen.findByTestId("fallback-llm")).toBeInTheDocument();
    expect(screen.queryByTestId("fallback-asr")).toBeNull();
  });
});
