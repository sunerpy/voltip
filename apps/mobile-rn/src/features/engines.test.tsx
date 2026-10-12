// The phone's provider sheets (docs/dictation.md §3.3, §3.5): with a saved key the model list has
// every model the provider lists, headed with 测试连接's count and asked for again as it opens;
// Model Studio's sheet and its fallback note link the console page of free quota and usage.
import { fireEvent, screen, waitFor, within } from "@testing-library/react-native";
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
    // The sheet asks for the latest list as it opens.
    expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
      provider: "groq",
      kind: "asr",
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
});
