import { type Backend, type ProviderId, type ServiceKind, zhT } from "@voltip/shared";
import { MOCK_PROBE_MS, MockBackend } from "@voltip/shared/mock";
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { BackendProvider, useUiState } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { FeatureShellProvider } from "../shell";
import { ProviderCard } from "./ProviderCard";
import { PROBE_WAIT_MS, useProviderProbe } from "./useProviderProbe";

const t = zhT.t;

/** The card for `id` once the backend's state has loaded. */
function Card({
  id,
  kind,
  localBody,
}: {
  id: ProviderId;
  kind: ServiceKind;
  localBody?: ReactNode;
}) {
  const state = useUiState();
  const provider = state.engines.providers.find((p) => p.id === id);
  if (provider === undefined) return null;
  return (
    <ProviderCard
      provider={provider}
      kind={kind}
      open
      onToggle={() => undefined}
      localBody={localBody}
    />
  );
}

function renderCard(
  backend: Backend,
  props: { id: ProviderId; kind: ServiceKind; localBody?: ReactNode },
) {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: () => undefined }}>
          <Card {...props} />
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
  return notify;
}

describe("ProviderCard (desktop engines pane and phone settings)", () => {
  it("saves a vendor's model and key, deletes the key and resets the card", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = renderCard(backend, { id: "groq", kind: "asr" });
    const form = await screen.findByTestId("provider-form");
    await user.selectOptions(
      within(form).getByRole("combobox", { name: t("engines.field.model") }),
      t("engines.field.modelOther"),
    );
    await user.type(within(form).getByLabelText(t("engines.field.modelCustom")), "my-whisper");
    await user.type(within(form).getByLabelText(t("engines.field.key")), "gsk_test");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: expect.objectContaining({ groq: { asr_model: "my-whisper" } }),
      }),
    });
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "groq",
      kind: "asr",
      value: "gsk_test",
    });
    expect(notify).toHaveBeenLastCalledWith(
      `${t("engines.saved", { provider: t("engines.provider.groq") })}${t("engines.savedKey")}`,
    );
    const deleteKey = await screen.findByRole("button", { name: t("engines.deleteKey") });
    await user.click(deleteKey);
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "groq",
      kind: "asr",
      value: null,
    });
    expect(notify).toHaveBeenLastCalledWith(
      t("engines.keyDeleted", { provider: t("engines.provider.groq") }),
    );
    await user.click(screen.getByRole("button", { name: t("engines.reset") }));
    expect(invoke).toHaveBeenLastCalledWith("settings_set_engines", {
      engines: expect.not.objectContaining({ providers: expect.anything() }),
    });
    expect(notify).toHaveBeenLastCalledWith(
      t("engines.resetDone", { provider: t("engines.provider.groq") }),
    );
    backend.destroy();
  });

  it("offers Model Studio's realtime model first and explains its workspace address", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    renderCard(backend, { id: "aliyun", kind: "asr" });
    const form = await screen.findByTestId("provider-form");
    const model = within(form).getByRole("combobox", { name: t("engines.field.model") });
    expect(model).toHaveValue("qwen-audio-3.1-asr-flash-streaming");
    expect(
      within(model)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([
      "qwen-audio-3.1-asr-flash-streaming",
      "qwen-audio-3.1-asr-flash",
      "qwen-audio-3.1-asr-flash-message",
      "qwen3-asr-flash",
      "fun-asr-realtime",
      t("engines.field.modelOther"),
    ]);
    expect(form).toHaveTextContent(t("engines.field.baseUrlHelpAliyun"));
    const url = within(form).getByLabelText(t("engines.field.baseUrl"));
    expect(url).toHaveAttribute("placeholder", "https://dashscope.aliyuncs.com/compatible-mode/v1");
    await user.type(url, "https://ws-test.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
    await user.type(within(form).getByLabelText(t("engines.field.key")), "sk-test");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: expect.objectContaining({
          aliyun: { asr_url: "https://ws-test.cn-beijing.maas.aliyuncs.com/compatible-mode/v1" },
        }),
      }),
    });
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "aliyun",
      kind: "asr",
      value: "sk-test",
    });
    backend.destroy();
  });

  it("docs/dictation.md §3.7: the custom clean-up card chooses Responses and an effort; others have no such fields", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    renderCard(backend, { id: "custom", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    const api = within(form).getByRole("combobox", { name: t("engines.field.api") });
    expect(api).toHaveValue("chat_completions");
    expect(within(form).queryByRole("combobox", { name: t("engines.field.reasoning") })).toBeNull();
    await user.selectOptions(api, "Responses");
    const effort = within(form).getByRole("combobox", { name: t("engines.field.reasoning") });
    expect(effort).toHaveValue("");
    expect(screen.getByTestId("provider-api-help")).toHaveTextContent(
      t("engines.field.reasoningHelp"),
    );
    await user.selectOptions(effort, "high");
    await user.type(
      within(form).getByLabelText(t("engines.field.baseUrl")),
      "http://127.0.0.1:8787/v1",
    );
    // The custom provider has no model list: the field is a plain one.
    await user.type(within(form).getByLabelText(t("engines.field.modelCustom")), "claude-opus-5-5");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: expect.objectContaining({
          custom: {
            llm_url: "http://127.0.0.1:8787/v1",
            llm_model: "claude-opus-5-5",
            llm_api: "responses",
            llm_reasoning: "high",
          },
        }),
      }),
    });
    backend.destroy();
  });

  it("shows no interface choice on a vendor's clean-up card or the custom recognition card", async () => {
    const backend = new MockBackend();
    renderCard(backend, { id: "openai", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    expect(within(form).queryByRole("combobox", { name: t("engines.field.api") })).toBeNull();
    expect(screen.queryByTestId("provider-api-help")).toBeNull();
    backend.destroy();
  });

  it("refuses a draft the checks reject and saves nothing", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = renderCard(backend, { id: "custom", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(screen.getByTestId("provider-problem")).toHaveTextContent(t("engines.check.missingUrl"));
    expect(invoke).not.toHaveBeenCalledWith("settings_set_engines", expect.anything());
    expect(notify).not.toHaveBeenCalled();
    backend.destroy();
  });

  it("switches the service to a provider and opens its key page", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = renderCard(backend, { id: "groq", kind: "llm" });
    await user.click(await screen.findByRole("button", { name: t("engines.use") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({ llm_provider: "groq" }),
    });
    expect(notify).toHaveBeenCalledWith(
      t("engines.used", { provider: t("engines.provider.groq") }),
    );
    await user.click(screen.getByRole("button", { name: t("engines.getKey") }));
    expect(backend.consolesOpened).toEqual(["groq"]);
    backend.destroy();
  });

  it("tests the connection and offers the listed models", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      providerKeys: [{ provider: "groq", kind: "asr" }],
      probeModels: { groq: ["whisper-x"] },
    });
    renderCard(backend, { id: "groq", kind: "asr" });
    const form = await screen.findByTestId("provider-form");
    await user.click(within(form).getByRole("button", { name: t("engines.probe") }));
    expect(
      await screen.findByTestId("probe-result", {}, { timeout: MOCK_PROBE_MS * 20 }),
    ).toHaveTextContent(t("engines.probeResult.ok", { n: 1, ms: MOCK_PROBE_MS }));
    expect(within(form).getByRole("option", { name: "whisper-x" })).toBeInTheDocument();
    backend.destroy();
  });

  it("picks one of the built-in clean-up's models, the first kept as no choice", async () => {
    // User request 2026-10-08: the built-in service offers the Qwen and GPT-OSS models its
    // gateway serves.
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = renderCard(backend, { id: "builtin", kind: "llm" });
    const select = await screen.findByRole("combobox", { name: t("engines.field.model") });
    expect([...select.querySelectorAll("option")].map((o) => o.value)).toEqual([
      "qwen/qwen3.8-27b",
      "openai/gpt-oss-120b",
      "openai/gpt-oss-20b",
    ]);
    await user.selectOptions(select, "openai/gpt-oss-120b");
    expect(invoke).toHaveBeenLastCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: { builtin: { llm_model: "openai/gpt-oss-120b" } },
      }),
    });
    expect(notify).toHaveBeenLastCalledWith(
      t("engines.saved", { provider: t("engines.provider.builtin") }),
    );
    await waitFor(async () => {
      expect((await backend.getState()).engines.refine_model).toBe("openai/gpt-oss-120b");
    });
    expect(screen.getByRole("combobox", { name: t("engines.field.model") })).toHaveValue(
      "openai/gpt-oss-120b",
    );
    await user.selectOptions(
      screen.getByRole("combobox", { name: t("engines.field.model") }),
      "qwen/qwen3.8-27b",
    );
    expect(invoke).toHaveBeenLastCalledWith("settings_set_engines", {
      engines: expect.not.objectContaining({ providers: expect.anything() }),
    });
    // Recognition offers one model: named, not chosen.
    backend.destroy();
    const one = new MockBackend();
    renderCard(one, { id: "builtin", kind: "asr" });
    expect(
      await screen.findByText(t("engines.model", { model: "Qwen/Qwen3-ASR-1.7B" })),
    ).toBeInTheDocument();
    one.destroy();
  });

  it("offers Google AI Studio's Gemini models for polish, with the user's own key", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    renderCard(backend, { id: "google", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    const select = within(form).getByRole("combobox", { name: t("engines.field.model") });
    expect([...select.querySelectorAll("option")].map((o) => o.value)).toEqual([
      "gemini-3.8-flash",
      "gemini-3.5-flash-lite",
      "gemini-3.1-flash-lite",
      "gemini-2.5-flash",
      "gemini-2.5-pro",
      "__other__",
    ]);
    expect(screen.getByText(t("engines.providerNote.google"))).toBeInTheDocument();
    await user.selectOptions(select, "gemini-3.5-flash-lite");
    await user.type(within(form).getByLabelText(t("engines.field.key")), "AIza-test");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: expect.objectContaining({ google: { llm_model: "gemini-3.5-flash-lite" } }),
      }),
    });
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "google",
      kind: "llm",
      value: "AIza-test",
    });
    backend.destroy();
  });

  it("explains the built-in service and shows the shell's body for the on-device card", async () => {
    const backend = new MockBackend();
    renderCard(backend, { id: "builtin", kind: "asr" });
    expect(await screen.findByText(t("engines.builtinBody"))).toBeInTheDocument();
    expect(screen.queryByTestId("provider-form")).toBeNull();
    backend.destroy();
    const desktop = new MockBackend();
    renderCard(desktop, { id: "local", kind: "asr", localBody: <p>本地模型库</p> });
    expect(await screen.findByText("本地模型库")).toBeInTheDocument();
    desktop.destroy();
  });
});

describe("useProviderProbe", () => {
  function silent(invoke: Backend["invoke"]): Backend {
    const inner = new MockBackend();
    return Object.assign(Object.create(inner) as Backend, {
      invoke,
      on: () => () => undefined,
    });
  }

  function wrapper(backend: Backend) {
    return ({ children }: { children: ReactNode }) => (
      <BackendProvider backend={backend}>{children}</BackendProvider>
    );
  }

  it("says the test timed out when no answer comes", async () => {
    vi.useFakeTimers();
    try {
      const backend = silent(() => Promise.resolve(undefined as never));
      const { result } = renderHook(() => useProviderProbe("groq", "asr"), {
        wrapper: wrapper(backend),
      });
      act(() => {
        result.current.run({ baseUrl: " https://x.test/v1 ", key: " " });
      });
      expect(result.current.pending).toBe(true);
      act(() => {
        vi.advanceTimersByTime(PROBE_WAIT_MS);
      });
      expect(result.current.pending).toBe(false);
      expect(result.current.report).toEqual({
        provider: "groq",
        kind: "asr",
        result: "failed",
        reason: "timeout",
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("says the shell cannot test when the command is refused", async () => {
    const backend = silent(() => Promise.reject(new Error("no")));
    const { result } = renderHook(() => useProviderProbe("custom", "llm"), {
      wrapper: wrapper(backend),
    });
    act(() => {
      result.current.run({});
    });
    await waitFor(() => {
      expect(result.current.report?.reason).toBe("unsupported");
    });
    expect(result.current.pending).toBe(false);
  });
});
