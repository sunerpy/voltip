import { type ServiceKind, zhT } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { FeatureShellProvider } from "../shell";
import { FallbackSection } from "./FallbackSection";

const t = zhT.t;

function renderSection(backend: MockBackend, kind: ServiceKind = "asr") {
  const notify = vi.fn<(message: string, tone?: "neutral" | "danger") => void>();
  const view = render(
    <BackendProvider backend={backend}>
      <I18nProvider locale="zh-CN">
        <FeatureShellProvider shell={{ notify, confirm: () => undefined }}>
          <FallbackSection kind={kind} />
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

  it("moves and removes models, and shows the built-in service's own model", async () => {
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
    // The built-in service: its own model, nothing to type.
    const add = screen.getByTestId("fallback-llm-add");
    await user.selectOptions(
      within(add).getByRole("combobox", { name: t("engines.fallback.provider") }),
      "builtin",
    );
    expect(within(add).getByRole("textbox", { name: t("engines.fallback.model") })).toHaveAttribute(
      "readonly",
    );
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
