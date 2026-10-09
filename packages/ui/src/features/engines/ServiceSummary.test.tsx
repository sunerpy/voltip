import { type EngineSettings, type ServiceKind } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, render, screen, waitFor } from "@testing-library/react";
import { BackendProvider } from "../../backend/BackendProvider";
import { I18nProvider } from "../../i18n/I18nProvider";
import { CurrentService, ServicePrivacy } from "./ServiceSummary";

function renderSummary(backend: MockBackend, kind: ServiceKind, locale: "zh-CN" | "en" = "zh-CN") {
  return render(
    <BackendProvider backend={backend}>
      <I18nProvider locale={locale}>
        <CurrentService kind={kind} />
        <ServicePrivacy kind={kind} />
      </I18nProvider>
    </BackendProvider>,
  );
}

async function withEngines(backend: MockBackend, patch: Partial<EngineSettings>) {
  const state = await backend.getState();
  await backend.invoke("settings_set_engines", {
    engines: { ...state.settings.engines, ...patch },
  });
}

const GROQ_ASR = { provider: "groq" as const, model: "whisper-large-v3" };

describe("the current and privacy lines (docs/dictation.md §3.5)", () => {
  it("name the selected model, and a fallback model standing in for it", async () => {
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    await withEngines(backend, { asr_fallback: { enabled: true, models: [GROQ_ASR] } });
    const { unmount } = renderSummary(backend, "asr");
    await waitFor(() => {
      expect(screen.getByTestId("current-asr")).toHaveTextContent(
        "当前：内置服务 · Qwen3-ASR-1.7B",
      );
    });
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent(
      "音频发送到内置服务；额度用完时改发给Groq",
    );
    act(() => {
      backend.simulateQuotaExhausted("asr", Date.now() + 86_400_000);
    });
    await waitFor(() => {
      expect(screen.getByTestId("current-asr")).toHaveTextContent(
        "当前：Groq · whisper-large-v3（候补）",
      );
    });
    expect(screen.getByTestId("privacy-asr")).toHaveTextContent("额度用完时改发给Groq");
    unmount();
  });

  it("name nobody else with the switch off or a selected service the chain does not run for", async () => {
    const backend = new MockBackend({ providerKeys: [{ provider: "groq", kind: "asr" }] });
    await withEngines(backend, { asr_fallback: { enabled: false, models: [GROQ_ASR] } });
    const { unmount } = renderSummary(backend, "asr");
    await waitFor(() => {
      expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到内置服务");
    });
    expect(screen.getByTestId("privacy-asr")).not.toHaveTextContent("额度用完");
    await act(async () => {
      await withEngines(backend, {
        asr_provider: "openai",
        asr_fallback: { enabled: true, models: [GROQ_ASR] },
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频发送到OpenAI");
    });
    expect(screen.getByTestId("privacy-asr")).not.toHaveTextContent("额度用完");
    expect(screen.getByTestId("current-asr")).toHaveTextContent("当前：OpenAI");
    unmount();
  });

  it("keep the audio on the computer for a local model, and say nothing without a clean-up provider", async () => {
    const backend = new MockBackend({ builtIn: {} });
    await withEngines(backend, { asr_provider: "local" });
    const { unmount } = renderSummary(backend, "asr");
    await waitFor(() => {
      expect(screen.getByTestId("privacy-asr")).toHaveTextContent("音频不离开本机");
    });
    unmount();
    const view = renderSummary(backend, "llm");
    await waitFor(() => {
      expect(screen.getByTestId("current-llm")).toHaveTextContent("当前：未选择");
    });
    expect(screen.queryByTestId("privacy-llm")).toBeNull();
    view.unmount();
  });

  it("list several fallback providers in the reader's language", async () => {
    const backend = new MockBackend({
      providerKeys: [
        { provider: "groq", kind: "llm" },
        { provider: "deepseek", kind: "llm" },
      ],
    });
    await withEngines(backend, {
      llm_fallback: {
        enabled: true,
        models: [
          { provider: "groq", model: "llama-3.3-70b-versatile" },
          { provider: "deepseek", model: "deepseek-chat" },
        ],
      },
    });
    const { unmount } = renderSummary(backend, "llm", "en");
    await waitFor(() => {
      expect(screen.getByTestId("privacy-llm")).toHaveTextContent(
        "to Groq, DeepSeek when the quota runs out",
      );
    });
    unmount();
  });
});
