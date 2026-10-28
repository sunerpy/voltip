// The phone's provider sheets (docs/dictation.md §3.3, §3.5): with a saved key the model list has
// every model the provider lists, headed with 测试连接's count and asked for again as it opens;
// Model Studio's sheet and its fallback note link the console page of free quota and usage.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { MOCK_PROBE_MS, MockBackend } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

const LONG = { timeout: MOCK_PROBE_MS * 20 };

describe("the phone's provider sheets", () => {
  it("regression: with a saved key the model list has every model the provider lists, under 测试连接's count", async () => {
    // User report 2026-10-12: the list had the few suggested models while 测试连接 counted many.
    const backend = new MockBackend({
      role: "phone",
      probeModels: { groq: ["whisper-x", "distil-whisper-y", "whisper-large-v3-turbo"] },
    });
    await backend.invoke("provider_key_set", { provider: "groq", kind: "asr", value: "gsk_saved" });
    const invoke = jest.spyOn(backend, "invoke");
    await renderApp({ backend });
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-speech"));
    const groq = await screen.findByTestId("provider-asr-groq");
    // A vendor's card shows its own mark (user request 2026-10-12); the built-in service its icon.
    expect(within(groq).getByTestId("provider-mark-groq")).toBeOnTheScreen();
    expect(
      within(screen.getByTestId("provider-asr-builtin")).queryByTestId(/^provider-mark-/),
    ).toBeNull();
    await fireEvent.press(within(groq).getByRole("button", { name: "Groq" }));
    const sheet = await screen.findByTestId("provider-sheet-sheet");
    // The sheet asks for the latest list as it opens, from an effect: the test waits for the call.
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
        provider: "groq",
        kind: "asr",
      });
    });
    await waitFor(() => {
      expect(within(sheet).getByTestId("provider-model-help")).toHaveTextContent(
        "列表包含服务商提供的 3 个可用模型，每次打开列表时更新。",
      );
    }, LONG);
    await fireEvent.press(within(sheet).getByTestId("provider-probe"));
    await waitFor(() => {
      expect(within(sheet).getByTestId("probe-result")).toHaveTextContent(
        `连接正常 · 3 个模型 · ${MOCK_PROBE_MS} ms`,
      );
    }, LONG);
    // Each time the list opens it is asked again; the provider's models are a group of their own.
    invoke.mockClear();
    await fireEvent.press(within(sheet).getByTestId("provider-model"));
    expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
      provider: "groq",
      kind: "asr",
    });
    const list = await screen.findByTestId("provider-model-sheet");
    expect(within(list).getByText("服务商提供（3）")).toBeOnTheScreen();
    expect(within(list).getByTestId("provider-model-option-whisper-x")).toBeOnTheScreen();
    // A suggested model the provider lists shows in both groups, checked once.
    expect(
      within(list).getByTestId("provider-model-again-option-whisper-large-v3-turbo"),
    ).toBeOnTheScreen();
    await fireEvent.press(within(list).getByTestId("provider-model-option-whisper-x"));
    await fireEvent.press(within(sheet).getByTestId("provider-save"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
        engines: expect.objectContaining({ providers: { groq: { asr_model: "whisper-x" } } }),
      });
    });
  });

  it("Model Studio's sheet and fallback note link its page of free quota and usage", async () => {
    const backend = new MockBackend({ role: "phone" });
    await backend.invoke("provider_key_set", {
      provider: "aliyun",
      kind: "llm",
      value: "sk-saved",
    });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        llm_fallback: { enabled: true, models: [{ provider: "aliyun", model: "qwen3.8-flash" }] },
      },
    });
    await renderApp({ backend });
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-ai"));
    await fireEvent.press(await screen.findByTestId("fallback-llm-quota"));
    expect(backend.quotaPagesOpened).toEqual(["aliyun"]);
    const aliyun = await screen.findByTestId("provider-llm-aliyun");
    await fireEvent.press(within(aliyun).getByRole("button", { name: "阿里云百炼" }));
    const sheet = await screen.findByTestId("provider-sheet-sheet");
    expect(within(sheet).getByTestId("provider-quota-help")).toHaveTextContent(
      /百炼不提供查询接口/,
    );
    await fireEvent.press(within(sheet).getByTestId("provider-quota"));
    expect(backend.quotaPagesOpened).toEqual(["aliyun", "aliyun"]);
  });

  it("Kimi's sheet switches between its China and international sites (docs/dictation.md §3.2)", async () => {
    const backend = new MockBackend({ role: "phone" });
    await renderApp({ backend });
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-ai"));
    expect(
      within(await screen.findByTestId("provider-llm-glm")).getByTestId("provider-mark-glm"),
    ).toBeOnTheScreen();
    const kimi = await screen.findByTestId("provider-llm-kimi");
    await fireEvent.press(within(kimi).getByRole("button", { name: "Kimi" }));
    const sheet = await screen.findByTestId("provider-sheet-sheet");
    expect(within(sheet).getByTestId("provider-site")).toHaveTextContent(
      /两个站点的账号和密钥不通用/,
    );
    await fireEvent.press(within(sheet).getByTestId("provider-site-intl"));
    expect(within(sheet).getByTestId("provider-url")).toHaveProp(
      "value",
      "https://api.moonshot.ai/v1",
    );
    await fireEvent.press(within(sheet).getByText("获取密钥"));
    expect(backend.consolesOpened).toEqual(["kimi"]);
    expect(backend.consoleSites).toEqual(["intl"]);
    await fireEvent.press(within(sheet).getByTestId("provider-site-cn"));
    expect(within(sheet).getByTestId("provider-url")).toHaveProp("value", "");
  });
});

/** 设置 › `page`, then `provider`'s sheet for `kind`. */
async function openSheet(
  page: "settings-speech" | "settings-ai",
  kind: "asr" | "llm",
  provider: string,
  name: string,
) {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId(page));
  const card = await screen.findByTestId(`provider-${kind}-${provider}`);
  await fireEvent.press(within(card).getByRole("button", { name }));
  return within(await screen.findByTestId("provider-sheet-sheet"));
}

async function pick(testID: string, option: string) {
  await fireEvent.press(screen.getByTestId(testID));
  await fireEvent.press(await screen.findByTestId(`${testID}-option-${option}`));
}

describe("a provider's sheet", () => {
  it("the custom clean-up chooses its interface and, for Responses, the reasoning effort", async () => {
    const { backend } = await renderApp();
    const sheet = await openSheet("settings-ai", "llm", "custom", "自定义接口");
    expect(sheet.queryByTestId("provider-reasoning")).toBeNull();
    await fireEvent.press(sheet.getByText("Responses"));
    expect(sheet.getByText(/只随 Responses 请求发送/)).toBeOnTheScreen();
    await pick("provider-reasoning", "high");
    await fireEvent.changeText(sheet.getByTestId("provider-model-id"), "gpt-6-luna");
    await fireEvent.changeText(sheet.getByTestId("provider-url"), "http://192.168.1.20:8000/v1");
    await fireEvent.press(sheet.getByTestId("provider-save"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom).toMatchObject({
        llm_api: "responses",
        llm_reasoning: "high",
      });
    });
    // No effort: none is sent; Chat Completions again drops both.
    await pick("provider-reasoning", "");
    await fireEvent.press(sheet.getByTestId("provider-save"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom?.llm_reasoning).toBeUndefined();
    });
    await fireEvent.press(sheet.getByText("Chat Completions"));
    expect(sheet.queryByTestId("provider-reasoning")).toBeNull();
    await fireEvent.press(sheet.getByTestId("provider-save"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.providers?.custom?.llm_api).toBeUndefined();
    });
  });

  it("shows or hides a key, saves it with a note, deletes it, and 恢复默认 forgets the draft", async () => {
    await renderApp();
    const sheet = await openSheet("settings-speech", "asr", "groq", "Groq");
    const key = sheet.getByTestId("provider-key");
    expect(key).toHaveProp("secureTextEntry", true);
    await fireEvent.press(sheet.getByLabelText("显示密钥"));
    expect(sheet.getByTestId("provider-key")).toHaveProp("secureTextEntry", false);
    await fireEvent.press(sheet.getByLabelText("隐藏密钥"));
    expect(sheet.getByTestId("provider-key")).toHaveProp("secureTextEntry", true);
    await fireEvent.changeText(sheet.getByTestId("provider-key"), "gsk_saved");
    await fireEvent.press(sheet.getByTestId("provider-save"));
    expect(await screen.findByText("已保存 · Groq · 密钥已写入系统钥匙串")).toBeOnTheScreen();
    expect(sheet.getByTestId("provider-key")).toHaveDisplayValue("");
    await fireEvent.press(await sheet.findByTestId("provider-delete-key"));
    expect(await screen.findByText("已删除Groq的密钥")).toBeOnTheScreen();
    // A model by id the list does not have.
    await pick("provider-model", "__other__");
    await fireEvent.changeText(sheet.getByTestId("provider-model-id"), "whisper-x");
    await fireEvent.press(sheet.getByTestId("provider-reset"));
    expect(await screen.findByText("已恢复Groq的默认设置")).toBeOnTheScreen();
    expect(sheet.queryByTestId("provider-model-id")).toBeNull();
  });

  it("says when the core refuses to test a connection", async () => {
    const { backend } = await renderApp();
    const sheet = await openSheet("settings-speech", "asr", "groq", "Groq");
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("unsupported"));
    await fireEvent.press(sheet.getByTestId("provider-probe"));
    expect(await sheet.findByText("这个服务不能测试连接")).toBeOnTheScreen();
  });

  it("switches back to the built-in service from its sheet", async () => {
    const backend = new MockBackend({ role: "phone" });
    await backend.invoke("provider_key_set", { provider: "groq", kind: "asr", value: "gsk_saved" });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: { ...state.settings.engines, asr_provider: "groq" },
    });
    await renderApp({ backend });
    const sheet = await openSheet("settings-speech", "asr", "builtin", "内置服务");
    expect(
      sheet.getByText("内置服务的地址和密钥已包含在应用中，此处不显示，也无需填写。"),
    ).toBeOnTheScreen();
    expect(sheet.getByText("模型 Qwen/Qwen3-ASR-1.7B")).toBeOnTheScreen();
    await fireEvent.press(sheet.getByTestId("provider-use"));
    expect(await screen.findByText("已切换到内置服务")).toBeOnTheScreen();
    await waitFor(() => {
      expect(backend.peek().settings.engines.asr_provider).toBe("builtin");
    });
  });
});

describe("the service in use", () => {
  it("names the fallback model standing in, and who else may get the audio", async () => {
    const backend = new MockBackend({
      role: "phone",
      providerKeys: [{ provider: "groq", kind: "asr" }],
    });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_fallback: { enabled: true, models: [{ provider: "groq", model: "whisper-large-v3" }] },
      },
    });
    await renderApp({ backend });
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-speech"));
    const current = await screen.findByTestId("current-asr");
    expect(current).toHaveTextContent(/^当前：内置服务 · /);
    expect(screen.getByText(/；额度用完时改发给Groq$/)).toBeOnTheScreen();
    await act(async () => {
      backend.simulateQuotaExhausted("asr", Date.now() + 3_600_000);
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.getByTestId("current-asr")).toHaveTextContent(
        "当前：Groq · whisper-large-v3（候补）",
      );
    });
  });

  it("changes the Chinese script, and asks nothing for the one already chosen", async () => {
    const { backend } = await renderApp();
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId("settings-speech"));
    const script = within(await screen.findByTestId("chinese-script"));
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(script.getByText("简体"));
    expect(invoke).not.toHaveBeenCalled();
    await fireEvent.press(script.getByText("繁体"));
    await waitFor(() => {
      expect(backend.peek().settings.engines.chinese_script).toBe("traditional");
    });
  });
});
