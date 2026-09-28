import { type HistoryEntry } from "@voltip/shared";
import {
  MOCK_LIVE_SCRIPT,
  MOCK_LIVE_STEP_MS,
  MOCK_MIC_READY_MS,
  MOCK_STREAMING_MODEL_ID,
  MockBackend,
  type MockBackendOptions,
  sampleDevices,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

const CJK = /[一-鿿]/;
const NOW = 1_758_700_000_000;

/** Every route of the desktop app with its `<h1>` per locale: the sweeps below render each one. */
const ROUTES = [
  ["/", "首页", "Home"],
  ["/history", "历史记录", "History"],
  ["/dictionary", "词典", "Dictionary"],
  ["/rules", "规则", "Rules"],
  ["/devices", "手机", "Phone"],
  ["/overlay", "悬浮胶囊", "Overlay"],
  ["/settings/speech", "首页", "Home"],
  ["/settings/ai", "首页", "Home"],
  ["/settings/scene", "首页", "Home"],
  ["/settings/general", "首页", "Home"],
  ["/settings/appearance", "首页", "Home"],
  ["/settings/hotkey", "首页", "Home"],
  ["/settings/privacy", "首页", "Home"],
  ["/settings/about", "首页", "Home"],
  ["/onboarding?step=1", "设置向导 / 第 1 步", "Setup guide / step 1"],
  ["/onboarding?step=2", "设置向导 / 第 2 步", "Setup guide / step 2"],
  ["/onboarding?step=3", "设置向导 / 第 3 步", "Setup guide / step 3"],
  ["/onboarding?step=4", "设置向导 / 第 4 步", "Setup guide / step 4"],
] as const;

/** The English sweep renders every route but the overlay spec sheet (a `pnpm dev` view whose
 *  sample sentences are Chinese); the dictionary and rules pages render the core's lists, seeded in
 *  English below. */
const ENGLISH_ROUTES = ROUTES.filter(([path]) => path !== "/overlay");

/** An eyebrow that is only ASCII capitals / punctuation: `SETTINGS`, `DRY RUN`, `SELF-CHECK`. */
const ASCII_CAPS = /^[A-Z][A-Z0-9 &·/'-]{2,}$/;
/** A Chinese phrase followed by an English gloss: `语音识别 · ASR`, `访问令牌 · Bearer`. */
const CJK_THEN_GLOSS = /[一-鿿].*·\s*([A-Za-z][A-Za-z0-9.-]{2,})$/;
/** Product, platform and protocol names, code identifiers and keycaps that stay Latin in every
 *  locale (`权限 · Windows` is a proper noun, `语音识别 · ASR` was a gloss). */
const LATIN_TERMS = new Set([
  "Windows",
  "macOS",
  "Linux",
  "Android",
  "iOS",
  "ASR",
  "LLM",
  "API",
  "CSV",
  "TOML",
  "JSON",
  "Ctrl",
  "Alt",
  "Space",
  "Noise",
  "SenseVoice",
  "Paraformer",
  "Zipformer",
  "sherpa-onnx",
  "history.json",
  "settings.json",
]);

function eyebrowTexts(root: ParentNode): string[] {
  return [...root.querySelectorAll('[class~="eyebrow"]')]
    .map((el) => (el.textContent ?? "").trim())
    .filter((text) => text.length > 0 && text !== "·");
}

/** English sample history so the assertion below is about UI copy, not user text. */
function englishHistory(): HistoryEntry[] {
  const base = {
    refined: true,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    refine_model: "qwen/qwen3.8-27b",
    duration_ms: 4200,
    asr_ms: 412,
    refine_ms: 210,
    starred: false,
    mode: "whole_take" as const,
    kind: "dictation" as const,
  };
  return [
    // docs/dictation.md §19: a voice edit row, the detail on the History page (newest first).
    {
      ...base,
      id: "00000000-0000-4000-8000-000000000004",
      at_ms: NOW - 30_000,
      raw_text: "make it more formal",
      text: "Dear colleagues, the meeting moves to Thursday at ten.",
      outcome: { kind: "inserted", via: "paste" },
      kind: "edit",
      edit: { instruction: "make it more formal", selection: "meeting moved to thursday 10ish" },
    },
    {
      ...base,
      id: "00000000-0000-4000-8000-000000000001",
      at_ms: NOW - 60_000,
      raw_text: "move the helper into session assembly",
      text: "Move the helper into session_assembly.",
      outcome: { kind: "inserted", via: "paste" },
      starred: true,
    },
    {
      ...base,
      id: "00000000-0000-4000-8000-000000000002",
      at_ms: NOW - 86_400_000,
      raw_text: "meeting moved to thursday",
      text: "Meeting moved to Thursday.",
      outcome: { kind: "clipboard", reason: "target window lost focus" },
    },
    {
      ...base,
      id: "00000000-0000-4000-8000-000000000003",
      at_ms: NOW - 3 * 86_400_000,
      raw_text: "add a retry",
      text: "Add a retry.",
      refined: false,
      outcome: { kind: "failed", reason: "uipi_denied" },
    },
  ];
}

/** An English dictionary, rule list (docs/dictation.md §16) and scene list (§18) for the
 *  dictionary / rules pages and the 场景 settings group. */
const ENGLISH_VOCABULARY: Pick<MockBackendOptions, "dictionary" | "rules" | "scenes"> = {
  dictionary: [
    {
      id: "00000000-0000-4000-8000-000000000011",
      term: "Kubernetes",
      heard_as: ["cube or net ease"],
      enabled: true,
      source: { kind: "history", history_id: "00000000-0000-4000-8000-000000000001" },
      created_at_ms: NOW,
      updated_at_ms: NOW,
    },
  ],
  rules: [
    {
      id: "00000000-0000-4000-9000-000000000011",
      name: "ticket",
      kind: "regex",
      pattern: "ticket (\\d+)",
      replacement: "",
      case_sensitive: false,
      enabled: true,
      created_at_ms: NOW,
      updated_at_ms: NOW,
    },
  ],
  scenes: [
    {
      id: "00000000-0000-4000-a000-000000000011",
      name: "Chat",
      enabled: true,
      match: { apps: ["slack"], title_contains: ["standup"] },
      overrides: {
        refine_enabled: true,
        refine_style: "punctuation",
        output_mode: "streaming_final",
        language: "en",
        chinese_script: "as_is",
        prompt: "casual",
      },
      created_at_ms: NOW,
      updated_at_ms: NOW,
    },
    {
      id: "00000000-0000-4000-a000-000000000012",
      name: "Docs",
      enabled: false,
      match: { apps: ["winword"], title_contains: [] },
      overrides: {},
      created_at_ms: NOW,
      updated_at_ms: NOW,
    },
  ],
};

function englishBackend(models?: MockBackendOptions["models"]) {
  return new MockBackend({
    settings: { locale: "en" },
    devices: sampleDevices(1_758_700_000),
    history: englishHistory(),
    now: () => NOW,
    ...ENGLISH_VOCABULARY,
    ...(models === undefined ? {} : { models }),
  });
}

/** The streaming model on disk: live preview is ready. */
function withStreamingModel() {
  return {
    [MOCK_STREAMING_MODEL_ID]: {
      kind: "installed" as const,
      path: `~/.local/share/voltip/models/${MOCK_STREAMING_MODEL_ID}`,
      installed_at: 1_758_600_000,
    },
  };
}

/** Text the user (or the sample core) produced, never UI copy: excluded from the CJK sweep. The
 *  language pickers name each language in itself on purpose (endonyms, like `English`). */
function uiText(root: HTMLElement): string {
  const clone = root.cloneNode(true) as HTMLElement;
  for (const el of clone.querySelectorAll("[data-user-text], [data-endonyms]")) el.remove();
  return (clone.textContent ?? "").replaceAll("简体中文", "");
}

describe("locale", () => {
  it("regression: switching the locale to English re-renders the shell, home, history, engines, settings and onboarding without CJK text", async () => {
    for (const [path, , title] of ENGLISH_ROUTES) {
      const { unmount } = renderApp({ path, backend: englishBackend() });
      expect(await screen.findByRole("heading", { name: title, level: 1 })).toBeInTheDocument();
      // The microphone readout resolves asynchronously; wait so its text is part of the sweep.
      await screen.findByText("Fifine K669");
      await waitFor(() => expect(document.documentElement.lang).toBe("en-US"));
      const shell = document.body;
      const text = uiText(shell);
      const offending = text.match(new RegExp(`.{0,20}${CJK.source}.{0,20}`))?.[0];
      expect(
        offending === undefined ? undefined : `${path} still renders CJK: ${offending}`,
      ).toBeUndefined();
      // Eyebrows are UI copy too: every one of them is English under `en`.
      const eyebrows = eyebrowTexts(shell);
      expect(eyebrows.length).toBeGreaterThan(0);
      expect(eyebrows.filter((e) => CJK.test(e)).map((e) => `${path}: ${e}`)).toEqual([]);
      // Every title / aria-label / placeholder attribute is UI copy too.
      const attributes = [...shell.querySelectorAll("[title], [aria-label], [placeholder]")]
        .filter((el) => el.closest("[data-user-text]") === null)
        .flatMap((el) =>
          ["title", "aria-label", "placeholder"].map((attr) => el.getAttribute(attr) ?? ""),
        );
      expect(attributes.filter((value) => CJK.test(value)).map((v) => `${path}: ${v}`)).toEqual([]);
      unmount();
    }
  });

  it("regression: under zh-CN no page shows an ASCII-caps eyebrow or an English gloss after a Chinese title", async () => {
    // User 2026-09-25: with a language switch in place the UI must not mix the two languages —
    // no `SETTINGS` over 设置, no `LOCAL MODELS · 本地模型`, no `语音识别 · ASR`.
    const offending: string[] = [];
    for (const [path, title] of ROUTES) {
      const { unmount } = renderApp({ path, systemLanguage: "zh-CN" });
      expect(await screen.findByRole("heading", { name: title, level: 1 })).toBeInTheDocument();
      if (path === "/overlay") await screen.findByTestId("page-overlay");
      else await screen.findByText("Fifine K669");
      await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
      const labels = eyebrowTexts(document.body);
      expect(labels.length).toBeGreaterThan(0);
      // The settings dialog's group nav: the tabs carry the Chinese group name and nothing else.
      const nav = document.querySelector('[role="dialog"] nav');
      if (nav) {
        for (const tab of nav.querySelectorAll('[role="tab"]')) {
          const text = (tab.textContent ?? "").trim();
          // "AI" is how Chinese UIs say it (the owner named the group AI 模型, 2026-09-27; the
          // title bar's AI润色 too); any other Latin letter in a tab is an English gloss.
          if (/[A-Za-z]/.test(text.replace(/\bAI\b/g, ""))) offending.push(`${path} tab: ${text}`);
        }
        for (const mono of nav.querySelectorAll('[class~="mono"]')) {
          const text = (mono.textContent ?? "").trim();
          // The footer names the app and its version (`Voltip 0.3.0`): a product name, not prose.
          if (/[A-Za-z]{3,}/.test(text.replace(/^Voltip(?: [0-9][0-9A-Za-z.+-]*)?$/, ""))) {
            offending.push(`${path} nav mono: ${text}`);
          }
        }
      }
      for (const text of labels) {
        if (ASCII_CAPS.test(text)) offending.push(`${path} caps eyebrow: ${text}`);
        const gloss = text.match(CJK_THEN_GLOSS)?.[1];
        if (gloss !== undefined && !LATIN_TERMS.has(gloss)) {
          offending.push(`${path} gloss eyebrow: ${text}`);
        }
      }
      unmount();
    }
    expect(offending).toEqual([]);
  });

  it("regression: dictation failure codes are localized on the home page and the live pill", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = englishBackend();
      renderApp({ backend });
      await screen.findByRole("heading", { name: "Home", level: 1 });
      act(() => {
        backend.publish({
          type: "dictation",
          kind: "dictation",
          session: 1,
          phase: { phase: "failed", message: "asr: 503 upstream", code: "no_speech" },
        });
      });
      expect(await screen.findByTestId("home-phase")).toHaveTextContent(
        "Failed · No speech detected",
      );
      act(() => {
        backend.publish({
          type: "dictation",
          kind: "dictation",
          session: 2,
          phase: { phase: "failed", message: "raw core message", code: "unknown", text: "kept" },
        });
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("Not inserted · raw core message");
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: the model tiers, the 实时预览 block, the home chip and the live pill render in English under en", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      // Settings › Speech models: the core's Chinese tier names become the dictionary's English names.
      const settings = englishBackend(withStreamingModel());
      const { unmount } = renderApp({ path: "/settings/speech", backend: settings });
      const localToggle = (await screen.findByTestId("provider-asr-local")).querySelector(
        "button[aria-expanded]",
      );
      if (!(localToggle instanceof HTMLElement)) throw new Error("local card toggle");
      act(() => {
        localToggle.click();
      });
      const library = await screen.findByRole("list", { name: "Local model library" });
      expect(
        within(library)
          .getAllByRole("article")
          .map((a) => a.getAttribute("aria-label")),
      ).toEqual(["Balanced", "Accurate", "Light", "Light · Chinese"]);
      expect(within(library).getByText("qwen3-asr-0.6b · transcribe.cpp")).toBeInTheDocument();
      expect(within(library).getByText(/^Recommended; Qwen3-ASR 0\.6B/)).toBeInTheDocument();
      act(() => {
        screen.getByRole("radio", { name: "Recognition" }).click();
      });
      const live = await screen.findByTestId("live-preview");
      expect(within(live).getByText("Live preview", { selector: ".eyebrow" })).toBeInTheDocument();
      expect(within(live).getByTestId("live-preview-state")).toHaveTextContent("Ready");
      expect(within(live).getByRole("switch", { name: "Live preview" })).toBeChecked();
      expect(within(live).getByRole("article", { name: "Live preview" })).toBeInTheDocument();
      expect(
        within(live).getByText(`${MOCK_STREAMING_MODEL_ID} · Zipformer streaming`),
      ).toBeInTheDocument();
      expect(uiText(document.body)).not.toMatch(CJK);
      unmount();
      // Home: the readiness chip.
      const home = englishBackend(withStreamingModel());
      const homeView = renderApp({ backend: home });
      await screen.findByRole("heading", { name: "Home", level: 1 });
      expect(screen.getByTestId("home-live-preview")).toHaveTextContent("Live preview");
      act(() => {
        home.publish({
          type: "dictation",
          kind: "dictation",
          session: 1,
          phase: {
            phase: "listening",
            started_at: NOW,
            ready: true,
            live: { committed: [], current: "hello", degraded: "open failed", injected: 0 },
            locked: false,
          },
        });
      });
      expect(screen.getByTestId("home-live-degraded")).toHaveTextContent(
        "Live preview stopped · the final text is unaffected",
      );
      expect(uiText(document.body)).not.toMatch(CJK);
      homeView.unmount();
      // The live pill: waiting for the microphone, then the two-tone caption with its chip.
      const pillBackend = new MockBackend({
        settings: { locale: "en" },
        now: () => Date.now(),
        models: withStreamingModel(),
      });
      renderApp({ path: "/overlay?state=live", backend: pillBackend });
      await screen.findByTestId("overlay-window");
      await act(async () => {
        await pillBackend.invoke("dictation_start");
      });
      expect(screen.getByTestId("pill-waiting")).toHaveTextContent("Waiting for mic");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS + 10);
      });
      expect(screen.getByTestId("pill-live")).toHaveTextContent("Preview");
      expect(screen.getByTestId("pill-live-current")).toHaveTextContent(
        MOCK_LIVE_SCRIPT[0]?.current ?? "",
      );
      // The sample sentence is user text; everything else on the pill is English.
      const pill = screen.getByRole("status");
      expect(pill.textContent?.replace(MOCK_LIVE_SCRIPT[0]?.current ?? "", "")).not.toMatch(CJK);
      expect(pill).toHaveTextContent("Cloud");
    } finally {
      vi.useRealTimers();
    }
  });

  it("follows settings.locale at runtime: the General pane switches the language through the core and <html lang> follows", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/general" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
    expect(within(dialog).getByRole("heading", { name: "通用", level: 2 })).toBeInTheDocument();
    const group = within(dialog).getByRole("radiogroup", { name: "语言" });
    expect(within(group).getByRole("radio", { name: "跟随系统" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(within(group).getByRole("radio", { name: "English" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("en");
    });
    // The whole app re-renders in English: dialog, nav, title bar, footer.
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "General", level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Home", level: 1 })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Main navigation" })).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.lang).toBe("en-US"));
    expect(screen.getByRole("radio", { name: "Follow system" })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "简体中文" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("zh-cn");
    });
    expect(await screen.findByRole("dialog", { name: "设置" })).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
    // `system` on an English OS resolves to English.
    await user.click(screen.getByRole("radio", { name: "跟随系统" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("system");
    });
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
  });

  it("system locale follows the OS language", async () => {
    renderApp({ systemLanguage: "en-GB" });
    expect(await screen.findByRole("heading", { name: "Home", level: 1 })).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.lang).toBe("en-US"));
  });
});
