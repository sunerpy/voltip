// 额度用完后改用其他模型 on the phone's 语音模型 and AI 模型 pages (docs/dictation.md §3.5): the switch,
// the chain with what became of each model, moving, removing and adding models, 重新检查 once the
// selected model ran out, and why the chain is not used.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type FallbackSettings, type ServiceKind } from "@voltip/shared";
import { MockBackend, type MockBackendOptions } from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";

const PAGE: Record<ServiceKind, string> = { asr: "settings-speech", llm: "settings-ai" };

/** The phone's core with `fallback` saved for `kind`. */
async function core(
  kind: ServiceKind,
  fallback: FallbackSettings,
  options: MockBackendOptions = {},
  engines: Record<string, unknown> = {},
) {
  const backend = new MockBackend({ role: "phone", ...options });
  const state = await backend.getState();
  await backend.invoke("settings_set_engines", {
    engines: {
      ...state.settings.engines,
      ...engines,
      ...(kind === "asr" ? { asr_fallback: fallback } : { llm_fallback: fallback }),
    },
  });
  return backend;
}

async function open(kind: ServiceKind, backend: MockBackend) {
  await renderApp({ backend });
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId(PAGE[kind]));
  return within(await screen.findByTestId(`fallback-${kind}`));
}

function saved(backend: MockBackend, kind: ServiceKind) {
  const engines = backend.peek().settings.engines;
  return kind === "asr" ? engines.asr_fallback : engines.llm_fallback;
}

/** Choose `option` in the picker `testID`. */
async function choose(testID: string, option: string) {
  await fireEvent.press(screen.getByTestId(testID));
  await fireEvent.press(
    await within(await screen.findByTestId(`${testID}-sheet`)).findByRole("radio", {
      name: option,
    }),
  );
}

describe("额度用完后改用其他模型", () => {
  it("switches on, adds a provider's model or another by id, and refuses a repeat or a blank one", async () => {
    const backend = new MockBackend({
      role: "phone",
      providerKeys: [{ provider: "aliyun", kind: "asr" }],
    });
    const section = await open("asr", backend);
    expect(section.getByText("尚未添加候补模型。")).toBeOnTheScreen();
    expect(section.getByText("当前选择")).toBeOnTheScreen();
    await fireEvent.press(section.getByTestId("fallback-asr-toggle"));
    await waitFor(() => {
      expect(saved(backend, "asr")?.enabled).toBe(true);
    });
    const add = within(section.getByTestId("fallback-asr-add"));
    // The on-device provider is never a fallback model.
    await fireEvent.press(add.getByTestId("fallback-asr-provider"));
    const providers = within(await screen.findByTestId("fallback-asr-provider-sheet"));
    expect(providers.queryByRole("radio", { name: "本机" })).toBeNull();
    await fireEvent.press(providers.getByRole("radio", { name: "阿里云百炼" }));
    await fireEvent.press(add.getByTestId("fallback-asr-add-button"));
    await waitFor(() => {
      expect(saved(backend, "asr")?.models).toEqual([
        { provider: "aliyun", model: "qwen-audio-3.1-asr-flash-streaming" },
      ]);
    });
    expect(await section.findByText(/^阿里云百炼：请在百炼控制台/)).toBeOnTheScreen();
    await fireEvent.press(section.getByTestId("fallback-asr-quota"));
    expect(backend.quotaPagesOpened).toEqual(["aliyun"]);
    // The same again is refused, with the reason.
    await fireEvent.press(add.getByTestId("fallback-asr-add-button"));
    expect(await add.findByRole("alert")).toHaveTextContent("这个模型已在列表中");
    // Any other id; a blank one is refused, and typing clears the reason.
    await fireEvent.press(add.getByTestId("fallback-asr-model"));
    await fireEvent.press(await screen.findByTestId("fallback-asr-model-option-__other__"));
    expect(add.queryByRole("alert")).toBeNull();
    await fireEvent.press(add.getByTestId("fallback-asr-add-button"));
    expect(await add.findByRole("alert")).toHaveTextContent("请选择或填写模型");
    await fireEvent.changeText(add.getByTestId("fallback-asr-custom"), " fun-asr-realtime ");
    expect(add.queryByRole("alert")).toBeNull();
    await fireEvent.press(add.getByTestId("fallback-asr-add-button"));
    await waitFor(() => {
      expect(saved(backend, "asr")?.models.map((m) => m.model)).toEqual([
        "qwen-audio-3.1-asr-flash-streaming",
        "fun-asr-realtime",
      ]);
    });
    // Back to a listed model of another provider: the typed id is gone.
    await choose("fallback-asr-provider", "Groq");
    expect(add.queryByTestId("fallback-asr-custom")).toBeNull();
  });

  it("moves and removes models, says which one has no key, and offers the built-in service's own models", async () => {
    const backend = await core(
      "llm",
      {
        enabled: true,
        models: [
          { provider: "groq", model: "llama-3.3-70b-versatile" },
          { provider: "openai", model: "gpt-6-luna" },
        ],
      },
      { providerKeys: [{ provider: "groq", kind: "llm" }] },
    );
    const section = await open("llm", backend);
    const list = within(section.getByTestId("fallback-llm-list"));
    expect(await list.findByText("缺少密钥")).toBeOnTheScreen();
    expect(list.getByLabelText("上移 llama-3.3-70b-versatile")).toBeDisabled();
    expect(list.getByLabelText("下移 gpt-6-luna")).toBeDisabled();
    await fireEvent.press(list.getByLabelText("下移 llama-3.3-70b-versatile"));
    await waitFor(() => {
      expect(saved(backend, "llm")?.models.map((m) => m.provider)).toEqual(["openai", "groq"]);
    });
    await fireEvent.press(list.getByLabelText("上移 llama-3.3-70b-versatile"));
    await waitFor(() => {
      expect(saved(backend, "llm")?.models.map((m) => m.provider)).toEqual(["groq", "openai"]);
    });
    await fireEvent.press(list.getByLabelText("移除 gpt-6-luna"));
    await waitFor(() => {
      expect(saved(backend, "llm")?.models).toEqual([
        { provider: "groq", model: "llama-3.3-70b-versatile" },
      ]);
    });
    // The built-in service: the models its build offers, nothing to type.
    const add = within(section.getByTestId("fallback-llm-add"));
    await choose("fallback-llm-provider", "内置服务");
    await fireEvent.press(add.getByTestId("fallback-llm-builtin-model"));
    const models = within(await screen.findByTestId("fallback-llm-builtin-model-sheet"));
    expect(models.getAllByRole("radio").map((r) => r.props.accessibilityLabel as string)).toEqual([
      "qwen/qwen3.8-27b",
      "openai/gpt-oss-120b",
      "openai/gpt-oss-20b",
    ]);
    await fireEvent.press(models.getByRole("radio", { name: "openai/gpt-oss-20b" }));
    await fireEvent.press(add.getByTestId("fallback-llm-add-button"));
    await waitFor(() => {
      expect(saved(backend, "llm")?.models).toEqual([
        { provider: "groq", model: "llama-3.3-70b-versatile" },
        { provider: "builtin", model: "openai/gpt-oss-20b" },
      ]);
    });
    // Switching the chain off keeps the list.
    await fireEvent.press(section.getByTestId("fallback-llm-toggle"));
    await waitFor(() => {
      expect(saved(backend, "llm")?.enabled).toBe(false);
    });
    expect(saved(backend, "llm")?.models).toHaveLength(2);
  });

  it("names the built-in service's model when its build has only one", async () => {
    const backend = new MockBackend({
      role: "phone",
      builtIn: {
        asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true },
        llm: { model: "qwen/qwen3.8-27b", key: true },
      },
    });
    const section = await open("llm", backend);
    const add = within(section.getByTestId("fallback-llm-add"));
    await choose("fallback-llm-provider", "内置服务");
    const model = add.getByTestId("fallback-llm-builtin-fixed");
    expect(model).toHaveDisplayValue("qwen/qwen3.8-27b");
    expect(model).toHaveProp("editable", false);
  });

  it("says when the selected model ran out and when it is tried again; 重新检查 forgets it", async () => {
    const backend = await core(
      "asr",
      { enabled: true, models: [{ provider: "groq", model: "whisper-large-v3" }] },
      { providerKeys: [{ provider: "groq", kind: "asr" }] },
    );
    const section = await open("asr", backend);
    const list = within(section.getByTestId("fallback-asr-list"));
    expect(await list.findByText("使用中")).toBeOnTheScreen();
    expect(list.getByText("可用")).toBeOnTheScreen();
    expect(section.queryByTestId("fallback-asr-recheck")).toBeNull();
    await act(async () => {
      backend.simulateQuotaExhausted("asr", new Date(2026, 9, 5, 14, 5).getTime());
      await Promise.resolve();
    });
    expect(await list.findByText(/^额度已用完 · .*14:05 再试$/)).toBeOnTheScreen();
    expect(list.getByText("使用中")).toBeOnTheScreen();
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(section.getByTestId("fallback-asr-recheck"));
    expect(invoke).toHaveBeenCalledWith("engines_quota_reset", { kind: "asr" });
    expect(await screen.findByText("已清除额度记录，之后的请求从所选模型开始")).toBeOnTheScreen();
    await waitFor(() => {
      expect(list.queryByText(/额度已用完/)).toBeNull();
    });
  });

  it("explains why the chain is not used while the selected service is not set up", async () => {
    const backend = await core(
      "asr",
      { enabled: true, models: [{ provider: "builtin", model: "" }] },
      {},
      { asr_provider: "openai" },
    );
    const section = await open("asr", backend);
    expect(
      await section.findByText(
        "当前选择不可用。候补模型只在额度用完时启用，不替代未配置好的服务商。",
      ),
    ).toBeOnTheScreen();
  });
});
