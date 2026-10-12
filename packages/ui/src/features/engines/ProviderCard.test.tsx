import {
  type Backend,
  type ProviderId,
  type ServiceKind,
  defaultEngineSettings,
  zhT,
} from "@voltip/shared";
import { MOCK_PROBE_MS, MockBackend } from "@voltip/shared/mock";
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { BackendProvider, useUiState } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { FeatureShellProvider } from "../shell";
import { ProviderCards, ProviderDetail } from "./ProviderCard";
import { PROBE_WAIT_MS, useProviderProbe } from "./useProviderProbe";

const t = zhT.t;

/** The settings drawer of `id` once the backend's state has loaded, as a card opens it. */
function Detail({
  id,
  kind,
  localBody,
  onClose,
}: {
  id: ProviderId;
  kind: ServiceKind;
  localBody?: ReactNode;
  onClose: () => void;
}) {
  const state = useUiState();
  const provider = state.engines.providers.find((p) => p.id === id);
  if (provider === undefined) return null;
  return <ProviderDetail provider={provider} kind={kind} localBody={localBody} onClose={onClose} />;
}

/** The cards of every provider of `kind`. */
function Cards({ kind }: { kind: ServiceKind }) {
  const state = useUiState();
  const providers = state.engines.providers.filter((p) => p[kind] !== undefined);
  return <ProviderCards kind={kind} providers={providers} label="服务商" />;
}

function shell(backend: Backend, ui: ReactNode) {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: () => undefined }}>
          {ui}
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
  return notify;
}

function renderCard(
  backend: Backend,
  props: { id: ProviderId; kind: ServiceKind; localBody?: ReactNode },
) {
  const onClose = vi.fn();
  const notify = shell(backend, <Detail {...props} onClose={onClose} />);
  return Object.assign(notify, { onClose });
}

describe("ProviderCards (desktop model pages and phone settings)", () => {
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

  it("设为当前 in the drawer saves the draft, switches the service to the provider and closes", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = renderCard(backend, { id: "groq", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    // Groq needs its key: the checks refuse first, and nothing is switched.
    await user.click(within(form).getByRole("button", { name: t("engines.setCurrent") }));
    expect(within(form).getByTestId("provider-problem")).toHaveTextContent(
      t("engines.check.missingKey"),
    );
    expect(invoke).not.toHaveBeenCalledWith("settings_set_engines", expect.anything());
    expect(notify.onClose).not.toHaveBeenCalled();
    await user.type(within(form).getByLabelText(t("engines.field.key")), "gsk_test");
    await user.click(within(form).getByRole("button", { name: t("engines.setCurrent") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({ llm_provider: "groq" }),
    });
    expect(invoke).toHaveBeenCalledWith("provider_key_set", {
      provider: "groq",
      kind: "llm",
      value: "gsk_test",
    });
    expect(notify).toHaveBeenLastCalledWith(
      `${t("engines.used", { provider: t("engines.provider.groq") })}${t("engines.savedKey")}`,
    );
    expect(notify.onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: t("engines.getKey") }));
    expect(backend.consolesOpened).toEqual(["groq"]);
    backend.destroy();
  });

  it("a card names the provider, its host or what it is, its state and its model; 设为当前 switches without opening it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const invoke = vi.spyOn(backend, "invoke");
    const notify = shell(backend, <Cards kind="asr" />);
    const list = await screen.findByRole("list", { name: "服务商" });
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
    const builtin = screen.getByTestId("provider-asr-builtin");
    expect(builtin).toHaveAttribute("data-selected", "true");
    expect(builtin).toHaveTextContent("随应用提供，无需密钥");
    expect(within(builtin).getByText("使用中")).toBeInTheDocument();
    expect(within(builtin).queryByText("就绪")).toBeNull();
    expect(within(builtin).getByText(t("engines.isCurrent"))).toBeInTheDocument();
    expect(within(builtin).getByTestId("provider-model")).toHaveTextContent("Qwen/Qwen3-ASR-1.7B");
    // The in-use block is a soft accent one, never a solid fill (user feedback 2026-10-10).
    const block = builtin.querySelector("svg")?.parentElement;
    expect(block?.className).toContain("bg-accent-soft");
    expect(block?.className).not.toMatch(/(^|\s)bg-accent(\s|$)/);
    const groq = screen.getByTestId("provider-asr-groq");
    expect(groq).toHaveTextContent("api.groq.com");
    expect(within(groq).getByText("缺少密钥")).toBeInTheDocument();
    expect(within(groq).getByTestId("provider-model")).toHaveTextContent("whisper-large-v3-turbo");
    expect(screen.getByTestId("provider-asr-custom")).toHaveTextContent("OpenAI 兼容接口");
    expect(
      within(screen.getByTestId("provider-asr-local")).getByText("模型未下载"),
    ).toBeInTheDocument();
    await user.click(within(groq).getByRole("button", { name: t("engines.setCurrent") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({ asr_provider: "groq" }),
    });
    expect(notify).toHaveBeenCalledWith(t("engines.used", { provider: "Groq" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => {
      expect(screen.getByTestId("provider-asr-groq")).toHaveAttribute("data-selected", "true");
    });
    expect(screen.getByTestId("provider-asr-builtin")).not.toHaveAttribute("data-selected");
    backend.destroy();
  });

  it("a click on a card opens its drawer, which Esc closes, back on the card", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    shell(backend, <Cards kind="asr" />);
    const groq = await screen.findByTestId("provider-asr-groq");
    const open = within(groq).getByRole("button", { name: "Groq" });
    expect(open).toHaveAttribute("aria-haspopup", "dialog");
    expect(open).toHaveAttribute("aria-expanded", "false");
    await user.click(open);
    const drawer = screen.getByRole("dialog", { name: "Groq" });
    expect(drawer).toHaveAttribute("data-testid", "provider-asr-groq-detail");
    expect(drawer).toHaveAccessibleDescription(t("engines.providerNote.groq"));
    expect(within(drawer).getByText("缺少密钥")).toBeInTheDocument();
    expect(groq).toHaveAttribute("data-open", "true");
    expect(open).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(groq).not.toHaveAttribute("data-open");
    expect(open).toHaveFocus();
    // The on-device drawer is wider: its model cards come two to a row.
    await user.click(
      within(screen.getByTestId("provider-asr-local")).getByRole("button", { name: "本机" }),
    );
    const local = screen.getByRole("dialog", { name: "本机" });
    expect(local.style.width).toMatch(/^min\(720px, .*100vw/);
    // 设为当前 there switches to the on-device models.
    await user.click(within(local).getByRole("button", { name: t("engines.setCurrent") }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(async () => {
      expect((await backend.getState()).settings.engines.asr_provider).toBe("local");
    });
    backend.destroy();
  });

  it("regression: a long model id wraps in its own row, never beside the endpoint field", async () => {
    // User report 2026-10-10: the model select was never narrower than its longest option and
    // sat beside the endpoint field in one grid row, so a long id covered part of the address.
    const long = "FunAudioLLM/SenseVoiceSmall-2026-09-preview-int8-streaming";
    const backend = new MockBackend({
      settings: {
        engines: { ...defaultEngineSettings(), providers: { siliconflow: { asr_model: long } } },
      },
    });
    renderCard(backend, { id: "siliconflow", kind: "asr" });
    const form = await screen.findByTestId("provider-form");
    const model = within(form).getByRole("combobox", { name: t("engines.field.model") });
    expect(model).toHaveValue(long);
    // The chosen id is drawn in full on as many lines as it takes; no copy sets a width.
    expect(form.querySelector("[data-select-face]")).toHaveTextContent(long);
    expect(form.querySelector("[data-select-sizer]")).toBeNull();
    // One field per row: nothing in the drawer lays fields out in columns.
    for (const el of [form, ...form.querySelectorAll("[class]")])
      expect(el.getAttribute("class") ?? "").not.toMatch(/grid-template-columns|grid-cols-/);
    const url = within(form).getByLabelText(t("engines.field.baseUrl"));
    expect(model.closest("section")).not.toBe(url.closest("section"));
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

  it("regression: with a saved key the model list has every model the provider lists, under 测试连接's count", async () => {
    // User report 2026-10-12: the select offered the few suggested models while 测试连接 counted
    // many, and the user had to type a model id.
    const user = userEvent.setup();
    const listed = [
      "qwen/qwen3.8-27b",
      "openai/gpt-oss-120b",
      "moonshotai/kimi-k3",
      "meta/llama-5",
    ];
    const backend = new MockBackend({ probeModels: { groq: listed } });
    await backend.invoke("provider_key_set", { provider: "groq", kind: "llm", value: "gsk_saved" });
    const invoke = vi.spyOn(backend, "invoke");
    renderCard(backend, { id: "groq", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    // The drawer asks for the latest list as it opens.
    expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
      provider: "groq",
      kind: "llm",
    });
    const select = within(form).getByTestId("provider-model-select");
    const offered = await within(select).findByRole(
      "group",
      { name: t("engines.field.modelOffered", { n: 4 }) },
      { timeout: MOCK_PROBE_MS * 20 },
    );
    expect(
      within(offered)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["meta/llama-5", "moonshotai/kimi-k3", "openai/gpt-oss-120b", "qwen/qwen3.8-27b"]);
    // The suggested models first, then 其他模型…, then the provider's: what arrives goes last.
    expect(
      within(select)
        .getAllByRole("option")
        .slice(0, 4)
        .map((o) => o.textContent),
    ).toEqual([
      "qwen/qwen3.8-27b",
      "openai/gpt-oss-120b",
      "openai/gpt-oss-20b",
      t("engines.field.modelOther"),
    ]);
    expect(within(form).getByTestId("provider-model-help")).toHaveTextContent(
      "列表包含服务商提供的 4 个可用模型，每次打开列表时更新。",
    );
    // 测试连接 counts the same models.
    await user.click(within(form).getByRole("button", { name: t("engines.probe") }));
    expect(
      await screen.findByTestId("probe-result", {}, { timeout: MOCK_PROBE_MS * 20 }),
    ).toHaveTextContent(t("engines.probeResult.ok", { n: 4, ms: MOCK_PROBE_MS }));
    // Each time the list opens it is asked again, and a listed model can be chosen and saved.
    invoke.mockClear();
    await user.pointer({ keys: "[MouseLeft>]", target: select });
    expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
      provider: "groq",
      kind: "llm",
    });
    await user.selectOptions(select, "moonshotai/kimi-k3");
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", {
      engines: expect.objectContaining({
        providers: expect.objectContaining({ groq: { llm_model: "moonshotai/kimi-k3" } }),
      }),
    });
    backend.destroy();
  });

  it("a saved key's list follows a save, and Model Studio's drawer links its quota page", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      probeModels: { aliyun: ["qwen3.8-flash", "qwen-plus-latest"] },
    });
    const invoke = vi.spyOn(backend, "invoke");
    renderCard(backend, { id: "aliyun", kind: "llm" });
    const form = await screen.findByTestId("provider-form");
    // No key yet: the select has the suggestions only, and says where the rest come from.
    expect(within(form).getByTestId("provider-model-help")).toHaveTextContent(
      t("engines.field.modelHelp"),
    );
    await user.type(within(form).getByLabelText(t("engines.field.key")), "sk-test");
    invoke.mockClear();
    await user.click(within(form).getByRole("button", { name: t("engines.save") }));
    // The list is asked for once the key is stored, after it.
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
        provider: "aliyun",
        kind: "llm",
      });
    });
    const order = invoke.mock.calls.map(([name]) => name);
    expect(order.indexOf("provider_key_set")).toBeLessThan(
      order.indexOf("provider_models_refresh"),
    );
    expect(
      await within(form).findByRole(
        "group",
        { name: t("engines.field.modelOffered", { n: 2 }) },
        { timeout: MOCK_PROBE_MS * 20 },
      ),
    ).toBeInTheDocument();
    // docs/dictation.md §3.5: the quota is read in the console; the drawer says so and links it.
    expect(within(form).getByTestId("provider-quota-help")).toHaveTextContent(
      t("engines.quotaHelp"),
    );
    await user.click(within(form).getByTestId("provider-quota"));
    expect(backend.quotaPagesOpened).toEqual(["aliyun"]);
    backend.destroy();
  });

  it("a vendor's card and drawer show its own mark; the built-in, on-device and custom ones the app's icons", async () => {
    // User request 2026-10-12: the providers' real icons.
    const user = userEvent.setup();
    const backend = new MockBackend();
    shell(backend, <Cards kind="asr" />);
    const list = await screen.findByRole("list", { name: "服务商" });
    const marks = within(list)
      .getAllByRole("article")
      .map((card) => [
        card.getAttribute("aria-label"),
        card.querySelector("[data-mark]")?.getAttribute("data-mark") ??
          card.querySelector("[data-icon]")?.getAttribute("data-icon"),
      ]);
    expect(marks).toEqual([
      ["内置服务", "sparkles"],
      ["本机", "cpu"],
      ["OpenAI", "openai"],
      ["Groq", "groq"],
      ["硅基流动", "siliconflow"],
      ["阿里云百炼", "aliyun"],
      ["自定义接口", "link"],
    ]);
    // The marks are decorative: the provider's name says who it is.
    const groq = within(list).getByRole("article", { name: "Groq" });
    expect(groq.querySelector("[data-mark]")).toHaveAttribute("aria-hidden", "true");
    await user.click(within(groq).getByRole("button", { name: "Groq" }));
    const drawer = await screen.findByTestId("provider-asr-groq-detail");
    expect(drawer.querySelector("header [data-mark='groq']")).not.toBeNull();
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
