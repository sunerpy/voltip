import { type ServiceKind, zhT } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { FeatureShellProvider } from "../shell";
import { FallbackSection } from "./FallbackSection";

const t = zhT.t;

function renderSection(backend: MockBackend, kind: ServiceKind = "asr", outage = false) {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  const view = render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: () => undefined }}>
          <FallbackSection kind={kind} outage={outage} />
        </FeatureShellProvider>
      </I18nProvider>
    </BackendProvider>,
  );
  return { notify, unmount: view.unmount };
}

/** The chain's rows: their position, provider and model text, and state. */
function rows(kind: ServiceKind = "asr") {
  return within(screen.getByTestId(`fallback-${kind}-list`))
    .getAllByRole("listitem")
    .map((li) => [li.textContent ?? "", li.dataset.state]);
}

/** The `i`th 移除 button of the asr list. */
function nthRemove(i: number): HTMLElement {
  const button = screen.getAllByTestId("fallback-asr-remove")[i];
  if (button === undefined) throw new Error(`no 移除 ${String(i)}`);
  return button;
}

async function settingsFallback(backend: MockBackend, kind: ServiceKind) {
  const state = await backend.getState();
  return kind === "asr" ? state.settings.engines.asr_fallback : state.settings.engines.llm_fallback;
}

describe("FallbackSection (docs/dictation.md §3.5)", () => {
  it("switches on, adds models from a provider's presets or by id, and refuses repeats", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "aliyun", kind: "asr" }] });
    const invoke = vi.spyOn(backend, "invoke");
    const { unmount } = renderSection(backend);
    const section = await screen.findByTestId("fallback-asr");
    expect(section.dataset.enabled).toBe("false");
    expect(screen.getByTestId("fallback-asr-empty")).toHaveTextContent(t("engines.fallback.empty"));
    await waitFor(() => {
      expect(rows()).toHaveLength(1);
    });
    expect(rows()[0]?.[0]).toContain(t("engines.fallback.selected"));
    await user.click(screen.getByRole("switch", { name: t("engines.fallback.toggle") }));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "asr"))?.enabled).toBe(true);
    });
    await waitFor(() => {
      expect(screen.getByTestId("fallback-asr").dataset.inUse).toBe("true");
    });
    // 阿里云百炼 and its first preset; the on-device provider is never a fallback model.
    const add = screen.getByTestId("fallback-asr-add");
    const providers = within(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
    )
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(providers).toContain(t("engines.provider.aliyun"));
    expect(providers).not.toContain(t("engines.provider.local"));
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
      "aliyun",
    );
    await user.click(screen.getByTestId("fallback-asr-add-button"));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "asr"))?.models).toEqual([
        { provider: "aliyun", model: "qwen-audio-3.1-asr-flash-streaming" },
      ]);
    });
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "ready"]);
    });
    expect(screen.getByTestId("fallback-asr-aliyun")).toHaveTextContent("免费额度用完即停");
    // The same again: refused, with the reason.
    await user.click(screen.getByTestId("fallback-asr-add-button"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      t("engines.fallback.problem.listed"),
    );
    // Any other id; a blank one is refused.
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.model") }),
      t("engines.field.modelOther"),
    );
    await user.click(screen.getByTestId("fallback-asr-add-button"));
    expect(await screen.findByRole("alert")).toHaveTextContent(t("engines.fallback.problem.blank"));
    await user.type(
      screen.getByRole("textbox", { name: t("engines.field.modelCustom") }),
      "fun-asr-realtime",
    );
    await user.click(screen.getByTestId("fallback-asr-add-button"));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "asr"))?.models.map((m) => m.model)).toEqual([
        "qwen-audio-3.1-asr-flash-streaming",
        "fun-asr-realtime",
      ]);
    });
    expect(invoke).toHaveBeenCalledWith("settings_set_engines", expect.anything());
    unmount();
  });

  it("regression: the add row offers every model the chosen provider lists, and links Model Studio's quota page", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      providerKeys: [{ provider: "aliyun", kind: "llm" }],
      probeModels: { aliyun: ["qwen3.8-flash", "qwen-plus-latest", "deepseek-v4"] },
    });
    const invoke = vi.spyOn(backend, "invoke");
    const { unmount } = renderSection(backend, "llm");
    const add = await screen.findByTestId("fallback-llm-add");
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
      "aliyun",
    );
    // Choosing the provider asks for its list (from an effect, so the test waits for it); opening
    // the model list asks again.
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
        provider: "aliyun",
        kind: "llm",
      });
    });
    const model = within(add).getByTestId("fallback-llm-model");
    const offered = await within(model).findByRole(
      "group",
      { name: t("engines.field.modelOffered", { n: 3 }) },
      { timeout: 3000 },
    );
    expect(
      within(offered)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["deepseek-v4", "qwen-plus-latest", "qwen3.8-flash"]);
    invoke.mockClear();
    await user.pointer({ keys: "[MouseLeft>]", target: model });
    expect(invoke).toHaveBeenCalledWith("provider_models_refresh", {
      provider: "aliyun",
      kind: "llm",
    });
    await user.selectOptions(model, "qwen-plus-latest");
    await user.click(screen.getByTestId("fallback-llm-add-button"));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "llm"))?.models).toEqual([
        { provider: "aliyun", model: "qwen-plus-latest" },
      ]);
    });
    // docs/dictation.md §3.5: Model Studio's free quota and usage are read in its console.
    await user.click(await screen.findByTestId("fallback-llm-quota"));
    expect(backend.quotaPagesOpened).toEqual(["aliyun"]);
    unmount();
  });

  it("moves and removes models, and offers the built-in service's own models", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "llm" }] });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        llm_fallback: {
          enabled: true,
          models: [
            { provider: "groq", model: "llama-3.3-70b-versatile" },
            { provider: "openai", model: "gpt-6-luna" },
          ],
        },
      },
    });
    const { unmount } = renderSection(backend, "llm");
    await waitFor(() => {
      expect(rows("llm")).toHaveLength(3);
    });
    // The OpenAI model has no key: its row says so.
    expect(rows("llm")[2]).toEqual([
      expect.stringContaining(t("engines.issue.key_missing")),
      "issue",
    ]);
    await user.click(
      screen.getByRole("button", {
        name: t("engines.fallback.moveDown", { model: "llama-3.3-70b-versatile" }),
      }),
    );
    await waitFor(async () => {
      expect((await settingsFallback(backend, "llm"))?.models.map((m) => m.provider)).toEqual([
        "openai",
        "groq",
      ]);
    });
    expect(
      screen.getByRole("button", { name: t("engines.fallback.moveUp", { model: "gpt-6-luna" }) }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", {
        name: t("engines.fallback.moveUp", { model: "llama-3.3-70b-versatile" }),
      }),
    );
    await waitFor(async () => {
      expect((await settingsFallback(backend, "llm"))?.models.map((m) => m.provider)).toEqual([
        "groq",
        "openai",
      ]);
    });
    await user.click(
      screen.getByRole("button", { name: t("engines.fallback.remove", { model: "gpt-6-luna" }) }),
    );
    await waitFor(async () => {
      expect((await settingsFallback(backend, "llm"))?.models).toEqual([
        { provider: "groq", model: "llama-3.3-70b-versatile" },
      ]);
    });
    // The built-in service: the models its build offers, nothing to type.
    const add = screen.getByTestId("fallback-llm-add");
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
      "builtin",
    );
    const model = within(add).getByRole("combobox", { name: t("engines.fallback.model") });
    expect([...model.querySelectorAll("option")].map((o) => o.value)).toEqual([
      "qwen/qwen3.8-27b",
      "openai/gpt-oss-120b",
      "openai/gpt-oss-20b",
    ]);
    await user.selectOptions(model, "openai/gpt-oss-20b");
    await user.click(within(add).getByRole("button", { name: t("engines.fallback.add") }));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "llm"))?.models).toEqual([
        { provider: "groq", model: "llama-3.3-70b-versatile" },
        { provider: "builtin", model: "openai/gpt-oss-20b" },
      ]);
    });
    unmount();
  });

  it("regression: 添加 sits on the provider and model row, and every listed model shows 移除 in words (user request 2026-10-26)", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "aliyun", kind: "asr" }] });
    renderSection(backend);
    const add = await screen.findByTestId("fallback-asr-add");
    // One row: the provider, the model, then the button, in that order.
    const button = screen.getByTestId("fallback-asr-add-button");
    const provider = within(add).getByRole("combobox", { name: t("engines.fallback.provider") });
    const row = button.parentElement;
    if (row === null) throw new Error("row");
    expect(row).toContainElement(provider);
    expect(row).toHaveClass("flex", "flex-wrap", "items-end");
    expect(
      provider.compareDocumentPosition(button) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // Nothing listed: no 移除.
    expect(screen.queryAllByTestId("fallback-asr-remove")).toEqual([]);
    await user.selectOptions(provider, "aliyun");
    await user.click(button);
    await waitFor(() => {
      expect(screen.getAllByTestId("fallback-asr-remove")).toHaveLength(1);
    });
    const remove = nthRemove(0);
    // The word on the button, in the danger colour; its name says which model.
    expect(remove).toHaveTextContent(t("engines.fallback.removeShort"));
    expect(remove).toHaveClass("text-danger");
    expect(remove).toHaveAccessibleName(
      t("engines.fallback.remove", { model: "qwen-audio-3.1-asr-flash-streaming" }),
    );
    await user.click(remove);
    await waitFor(async () => {
      expect((await settingsFallback(backend, "asr"))?.models ?? []).toEqual([]);
    });
    expect(screen.queryAllByTestId("fallback-asr-remove")).toEqual([]);
  });

  it("names the built-in service's model when its build has only one", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      builtIn: {
        asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true },
        llm: { model: "qwen/qwen3.8-27b", key: true },
      },
    });
    const { unmount } = renderSection(backend, "llm");
    const add = await screen.findByTestId("fallback-llm-add");
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
      "builtin",
    );
    const model = within(add).getByRole("textbox", { name: t("engines.fallback.model") });
    expect(model).toHaveAttribute("readonly");
    expect(model).toHaveValue("qwen/qwen3.8-27b");
    unmount();
  });

  it("says when the selected model ran out and when it is tried again; 重新检查 forgets it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_fallback: { enabled: true, models: [{ provider: "groq", model: "whisper-large-v3" }] },
      },
    });
    const invoke = vi.spyOn(backend, "invoke");
    const { notify, unmount } = renderSection(backend);
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "ready"]);
    });
    expect(screen.queryByTestId("fallback-asr-recheck")).toBeNull();
    act(() => {
      backend.simulateQuotaExhausted("asr", new Date(2026, 9, 5, 14, 5).getTime());
    });
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["exhausted", "active"]);
    });
    expect(rows()[0]?.[0]).toContain("额度已用完");
    expect(rows()[0]?.[0]).toContain("14:05");
    await user.click(screen.getByTestId("fallback-asr-recheck"));
    expect(invoke).toHaveBeenCalledWith("engines_quota_reset", { kind: "asr" });
    expect(notify).toHaveBeenCalledWith(t("engines.fallback.recheckDone"));
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "ready"]);
    });
    unmount();
  });

  it("explains why the models are not used with an on-device selection", async () => {
    const backend = new MockBackend();
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_provider: "local",
        asr_fallback: { enabled: true, models: [{ provider: "builtin", model: "" }] },
      },
    });
    const { unmount } = renderSection(backend);
    expect(await screen.findByTestId("fallback-asr-not-in-use")).toHaveTextContent(
      t("engines.fallback.notInUse.local"),
    );
    unmount();
  });
});

describe("FallbackSection: 服务出错时也改用候补 (docs/dictation.md §3.14)", () => {
  const installed = { kind: "installed" as const, path: "/models/paraformer-zh", installed_at: 1 };

  it("is off by default, waits for the list's switch, and is saved like the list", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const { unmount } = renderSection(backend, "asr", true);
    const outage = await screen.findByRole("switch", { name: t("fallbackOutage.toggle") });
    expect(outage).toHaveAttribute("aria-checked", "false");
    expect(outage).toBeDisabled();
    expect(screen.getByTestId("fallback-asr-outage")).toHaveTextContent(t("fallbackOutage.help"));
    expect(screen.getByTestId("fallback-asr")).toHaveTextContent(
      t("engines.fallback.description.asr"),
    );
    await user.click(screen.getByRole("switch", { name: t("engines.fallback.toggle") }));
    await waitFor(() => {
      expect(screen.getByRole("switch", { name: t("fallbackOutage.toggle") })).toBeEnabled();
    });
    await user.click(screen.getByRole("switch", { name: t("fallbackOutage.toggle") }));
    await waitFor(async () => {
      expect(await settingsFallback(backend, "asr")).toMatchObject({
        enabled: true,
        on_outage: true,
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("fallback-asr").dataset.onOutage).toBe("true");
    });
    expect(screen.getByTestId("fallback-asr")).toHaveTextContent(
      t("fallbackOutage.description.asr"),
    );
    unmount();
    // The phone's section offers no such switch.
    const phone = renderSection(backend, "asr");
    await screen.findByTestId("fallback-asr");
    expect(screen.queryByRole("switch", { name: t("fallbackOutage.toggle") })).toBeNull();
    phone.unmount();
  });

  it("offers the downloaded models of this computer for the recognition, by name", async () => {
    const user = userEvent.setup();
    const none = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const first = renderSection(none, "asr", true);
    const providerSelect = async () =>
      within(await screen.findByTestId("fallback-asr-add")).getByRole("combobox", {
        name: t("engines.fallback.provider"),
      });
    expect(
      within(await providerSelect())
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).not.toContain(t("engines.provider.local"));
    first.unmount();

    const backend = new MockBackend({
      providerKeys: [{ provider: "groq", kind: "asr" }],
      models: { "paraformer-zh": installed },
    });
    const { unmount } = renderSection(backend, "asr", true);
    await user.selectOptions(await providerSelect(), "local");
    const models = within(screen.getByTestId("fallback-asr-add")).getAllByRole("combobox").at(-1);
    if (models === undefined) throw new Error("no model list");
    expect(
      within(models)
        .getAllByRole("option")
        .map((o) => o.getAttribute("value")),
    ).toEqual(["paraformer-zh"]);
    await user.click(screen.getByTestId("fallback-asr-add-button"));
    await waitFor(async () => {
      expect((await settingsFallback(backend, "asr"))?.models).toEqual([
        { provider: "local", model: "paraformer-zh" },
      ]);
    });
    await waitFor(() => {
      expect(rows()[1]?.[1]).toBe("ready");
    });
    expect(rows()[1]?.[0]).toContain(t("engines.provider.local"));
    unmount();
    // The clean-up has no models of this computer.
    const llm = renderSection(backend, "llm", true);
    const llmProviders = within(await screen.findByTestId("fallback-llm-add")).getByRole(
      "combobox",
      {
        name: t("engines.fallback.provider"),
      },
    );
    expect(
      within(llmProviders)
        .getAllByRole("option")
        .map((o) => o.getAttribute("value")),
    ).not.toContain("local");
    llm.unmount();
  });

  it("says a selected model of this computer moves on to models of this computer only", async () => {
    const backend = new MockBackend({
      providerKeys: [{ provider: "groq", kind: "asr" }],
      models: {
        "qwen3-asr-0.6b": { ...installed, path: "/models/qwen3-asr-0.6b" },
        "paraformer-zh": installed,
      },
    });
    const state = await backend.getState();
    const save = (on_outage: boolean, models: { provider: "groq" | "local"; model: string }[]) =>
      backend.invoke("settings_set_engines", {
        engines: {
          ...state.settings.engines,
          asr_provider: "local",
          local_model: "qwen3-asr-0.6b",
          asr_fallback: { enabled: true, on_outage, models },
        },
      });
    await save(false, [{ provider: "groq", model: "whisper-large-v3" }]);
    const { unmount } = renderSection(backend, "asr", true);
    expect(await screen.findByTestId("fallback-asr-not-in-use")).toHaveTextContent(
      t("fallbackOutage.notInUse.localOff"),
    );
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["ready", "not_for_local"]);
    });
    expect(rows()[1]?.[0]).toContain(t("fallbackOutage.state.notForLocal"));
    await act(async () => {
      await save(true, [{ provider: "groq", model: "whisper-large-v3" }]);
    });
    await waitFor(() => {
      expect(screen.getByTestId("fallback-asr-not-in-use")).toHaveTextContent(
        t("fallbackOutage.notInUse.localNone"),
      );
    });
    await act(async () => {
      await save(true, [
        { provider: "groq", model: "whisper-large-v3" },
        { provider: "local", model: "paraformer-zh" },
      ]);
    });
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "not_for_local", "ready"]);
    });
    expect(screen.queryByTestId("fallback-asr-not-in-use")).toBeNull();
    unmount();
  });

  it("shows a failed model skipped for now; 重新检查 clears both records", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    const state = await backend.getState();
    await backend.invoke("settings_set_engines", {
      engines: {
        ...state.settings.engines,
        asr_fallback: {
          enabled: true,
          on_outage: true,
          models: [{ provider: "groq", model: "whisper-large-v3" }],
        },
      },
    });
    const { notify, unmount } = renderSection(backend, "asr", true);
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "ready"]);
    });
    act(() => {
      backend.simulateServiceFailed("asr", new Date(2026, 9, 5, 14, 6).getTime());
    });
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["cooling", "active"]);
    });
    expect(rows()[0]?.[0]).toContain("出错，暂时跳过");
    expect(rows()[0]?.[0]).toContain("14:06");
    await user.click(screen.getByTestId("fallback-asr-recheck"));
    expect(notify).toHaveBeenCalledWith(t("fallbackOutage.recheckDone"));
    await waitFor(() => {
      expect(rows().map((r) => r[1])).toEqual(["active", "ready"]);
    });
    unmount();
  });
});
