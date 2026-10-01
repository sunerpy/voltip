import { defaultEngineSettings, joinLiveText, livePreviewText } from "@voltip/shared";
import {
  MOCK_ASR_MS,
  MOCK_COPY_MS,
  MOCK_DICTATION_DWELL_MS,
  MOCK_EDIT_TEXT,
  MOCK_DICTATION_TEXT,
  MOCK_FINALIZE_MS,
  MOCK_LIVE_SCRIPT,
  MOCK_LIVE_STEP_MS,
  MOCK_METER_INTERVAL_MS,
  MOCK_MIC_READY_MS,
  MOCK_REFINE_MS,
  MOCK_STREAMING_MODEL_ID,
  MockBackend,
} from "@voltip/shared/mock";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";
import { clipTail } from "@voltip/ui";
import {
  isOverlayWindowState,
  isPillState,
  pillLiveCaption,
  pillStateFor,
  stageStart,
} from "./Overlay";
import { fakeLevels } from "./OverlaySheet";

/** Fixed pixel panel sizes and two-fixed-column grids broke the 1440 / 1920 px windows (Windows
 *  test 2026-09-24); `max-w-[…]` / `min-w-[…]` caps stay allowed (the root is `max-w-[1440px]`). */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

/** Allow-list: the pills and the live caption (`role="status"`) are the overlay artefacts drawn at
 *  their real widths (52 / 260–340 / 420 px, 2 px waveform bars); the sheet around them is fluid. */
function isAllowedFixedSize(el: Element): boolean {
  return el.closest('[role="status"]') !== null;
}

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .filter((el) => !isAllowedFixedSize(el))
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

describe("Overlay page", () => {
  it("regression: overlay is fluid (no fixed-width panels)", async () => {
    renderApp({ path: "/overlay" });
    const page = await screen.findByTestId("page-overlay");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6");
    expect(fixedSizeOffenders(page)).toEqual([]);
    const [states, band] = [...page.querySelectorAll<HTMLElement>(":scope > .grid")];
    expect(states?.className).toMatch(/grid-cols-2 gap-4 lg:grid-cols-3/);
    expect(band?.className).toContain("grid-cols-1");
    expect(band?.className).toMatch(/lg:grid-cols-\[minmax\(320px,380px\)_minmax\(0,1fr\)\]/);
    // The pills themselves keep their real overlay geometry (that is what the sheet documents).
    expect(screen.getByText("预览").closest('[role="status"]')).toHaveClass("w-[420px]");
    // Bridge & MCP is being removed: nothing on the sheet names it.
    expect(page.textContent).not.toMatch(/Bridge|MCP/);
    // The processing pill names the preset, as the overlay does while AI polish runs (§21).
    const processing = page.querySelector<HTMLElement>('[data-state="processing"].rounded-pill');
    expect(processing?.textContent).toContain("校对");
    expect(processing?.textContent).not.toContain("AI 润色");
  });

  it("regression: the single-pill overlay window render is untouched by the sheet layout", async () => {
    renderApp({ path: "/overlay?state=inserted" });
    const win = await screen.findByTestId("overlay-window");
    expect(win).toHaveClass(
      "flex",
      "h-full",
      "items-start",
      "justify-center",
      "bg-transparent",
      "pt-2",
    );
    expect(win.children).toHaveLength(1);
    expect(win.firstElementChild).toHaveAttribute("data-state", "inserted");
    expect(screen.queryByTestId("page-overlay")).toBeNull();
  });

  it("renders the spec sheet's eight pill states, the live caption, toasts, fallback card and causes", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderApp({ path: "/overlay" });
    expect(await screen.findByRole("heading", { name: "悬浮窗", level: 1 })).toBeInTheDocument();
    await screen.findByTestId("page-overlay");
    const pills = screen.getAllByRole("status").filter((el) => el.hasAttribute("data-state"));
    expect(pills.map((p) => p.getAttribute("data-state"))).toEqual([
      "armed",
      "listening",
      "locked",
      "processing",
      "inserted",
      "error",
      "cancel-armed",
      "blocked",
    ]);
    expect(screen.getByText("预览")).toBeInTheDocument();
    expect(screen.getByText("文字没有送进目标窗口")).toBeInTheDocument();
    expect(screen.getByText("uipi_denied")).toBeInTheDocument();
    expect(screen.getByText("已取消 · 录音已丢弃")).toBeInTheDocument();
    expect(screen.queryByTestId("sample-data-notice")).toBeNull();
    expect(document.body.textContent).not.toMatch(/第二阶段|示例数据/);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    vi.useRealTimers();
  });

  it("regression: copy is real, the locked pill's 结束录音 is the real dictation_stop, demo undo does nothing and 打开历史 navigates", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ path: "/overlay" });
    await screen.findByRole("heading", { name: "悬浮窗", level: 1 });
    await screen.findByTestId("page-overlay");
    await user.click(screen.getAllByRole("button", { name: "复制文本" })[0] as HTMLElement);
    expect(writeText).toHaveBeenCalledWith("把 fetchUser 改成 async，然后加三次 retry。");
    expect(await screen.findByText("已复制到剪贴板 · 33 字")).toBeInTheDocument();
    // docs/dictation.md §13: a locked take is real now (hold_or_toggle short press), so the
    // sample pill's stop ends a running take through the core and never claims to be unwired.
    await act(async () => {
      await backend.invoke("dictation_start");
    });
    await user.click(screen.getByRole("button", { name: "结束录音" }));
    await waitFor(() => {
      expect(backend.peek().dictation.phase.phase).toBe("processing");
    });
    expect(screen.queryByText(/尚未接入/)).toBeNull();
    expect(screen.queryByText("已结束持续收音")).toBeNull();
    await user.click(screen.getAllByRole("button", { name: /撤销/ })[0] as HTMLElement);
    expect(screen.queryByText("录音已恢复，正在识别")).toBeNull();
    await user.click(screen.getByRole("button", { name: "打开历史" }));
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    expect(screen.queryByText("已在历史中打开")).toBeNull();
  });

  it("renders a single pill for the transparent overlay window and ignores unknown states", async () => {
    renderApp({ path: "/overlay?state=listening" });
    const win = await screen.findByTestId("overlay-window");
    expect(win.querySelector('[data-state="listening"]')).not.toBeNull();
    expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
    renderApp({ path: "/overlay?state=bogus" });
    expect(await screen.findByRole("heading", { name: "悬浮窗", level: 1 })).toBeInTheDocument();
    expect(isPillState("armed")).toBe(true);
    expect(isPillState("x")).toBe(false);
    expect(isOverlayWindowState("live")).toBe(true);
    expect(isOverlayWindowState("blank")).toBe(true);
    expect(isOverlayWindowState("listening")).toBe(true);
    expect(isOverlayWindowState("nope")).toBe(false);
    // A release build: the shell loads only `live` (and paints `blank`); sample pills are dev views.
    expect(isOverlayWindowState("live", false)).toBe(true);
    expect(isOverlayWindowState("blank", false)).toBe(true);
    expect(isOverlayWindowState("listening", false)).toBe(false);
    expect(pillStateFor({ phase: "idle" })).toBeUndefined();
    expect(pillStateFor({ phase: "listening", started_at: 0, ready: false, locked: false })).toBe(
      "listening",
    );
    expect(pillStateFor({ phase: "processing", stage: "inserting", started_at: 0 })).toBe(
      "processing",
    );
    expect(pillStateFor({ phase: "failed", message: "x" })).toBe("error");
    expect(pillStateFor({ phase: "cancelled", injected_chars: 0 })).toBe("cancel-armed");
    const levels = fakeLevels(3, 10);
    expect(levels).toHaveLength(10);
    expect(levels.every((l) => l >= 0 && l <= 1)).toBe(true);
  });

  it("regression: the prewarmed pill window (state=blank) paints nothing and never falls back to the spec sheet", async () => {
    renderApp({ path: "/overlay?state=blank" });
    const win = await screen.findByTestId("overlay-window");
    expect(win).toHaveAttribute("data-state", "blank");
    expect(win).toBeEmptyDOMElement();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByText(/悬浮胶囊 · 八态/)).toBeNull();
    expect(screen.queryByRole("navigation")).toBeNull();
  });
});

describe("Overlay in a release build", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.resetModules();
  });

  it("regression: a release build has no spec sheet and no sample pill; /overlay is a 404 and state=live still follows the core", async () => {
    vi.stubEnv("DEV", false);
    vi.resetModules();
    const release = await import("../test/render");
    const sheet = release.renderApp({ path: "/overlay" });
    expect(await screen.findByTestId("page-notfound")).toBeInTheDocument();
    expect(screen.queryByTestId("page-overlay")).toBeNull();
    sheet.unmount();
    const sample = release.renderApp({ path: "/overlay?state=listening" });
    expect(await screen.findByTestId("page-notfound")).toBeInTheDocument();
    expect(screen.queryByTestId("overlay-window")).toBeNull();
    sample.unmount();
    release.renderApp({ path: "/overlay?state=live" });
    const win = await screen.findByTestId("overlay-window");
    expect(win).toHaveAttribute("data-state", "blank");
    expect(screen.queryByRole("navigation")).toBeNull();
  });
});

describe("Overlay live window (state=live follows the core's dictation)", () => {
  it("regression: a phone's take leads the live pill with the phone's name while it records and processes (docs/dictation.md section 20)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({ now: () => Date.now() });
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      act(() => {
        backend.simulatePhoneTake("Pixel 8");
      });
      expect(await screen.findByTestId("pill-tag")).toHaveTextContent("手机 · Pixel 8");
      act(() => {
        backend.simulatePhoneTakeStop();
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("手机 · Pixel 8");
      // A take from the computer's own microphone carries no phone tag.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      expect(await screen.findByRole("status")).toHaveAttribute("data-state", "listening");
      expect(screen.queryByTestId("pill-tag")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: the live pill names what the take records, reads hours past an hour and counts a long take's segments while it records and processes (docs/dictation.md section 22)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      // By the backend's clock the device started 1:02:03 before the pill's.
      const backend = new MockBackend({ now: () => Date.now() - 3_723_000 });
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      await act(async () => {
        await backend.invoke("settings_set_recording", {
          recording: { source: "mixed", output_device: null, max_minutes: 120, echo_cancel: true },
        });
      });
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      expect(await screen.findByTestId("pill-source")).toHaveTextContent("混合");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS);
      });
      expect(screen.getByRole("status")).toHaveTextContent(/1:02:0\d/);
      expect(screen.queryByTestId("pill-progress")).toBeNull();
      act(() => {
        backend.simulateLongTakeProgress(12, 13);
      });
      expect(screen.getByTestId("pill-progress")).toHaveTextContent("已识别 12 段");
      await act(async () => {
        await backend.invoke("dictation_stop");
      });
      act(() => {
        backend.simulateLongTakeProgress(13, 14);
      });
      const processing = screen.getByRole("status");
      expect(processing).toHaveAttribute("data-state", "processing");
      expect(processing).toHaveTextContent("已识别 13/14 段");
      expect(screen.queryByTestId("pill-progress")).toBeNull();
      expect(screen.queryByTestId("pill-source")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: the live pill paints nothing while idle, then listening with the real meter, the processing stage, and the inserted count", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({ now: () => Date.now() });
      renderApp({ path: "/overlay?state=live", backend });
      const win = await screen.findByTestId("overlay-window");
      expect(win).toHaveAttribute("data-state", "blank");
      expect(screen.queryByRole("status")).toBeNull();
      expect(backend.activeMeters()).toBe(0);
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      const pill = await screen.findByRole("status");
      expect(pill).toHaveAttribute("data-state", "listening");
      expect(screen.getByTestId("overlay-window")).toHaveAttribute("data-session", "1");
      // Before the device delivered samples: the timer is parked and the hint says so (§11).
      expect(pill).toHaveTextContent("00:00");
      expect(pill).toHaveTextContent("等待麦克风");
      expect(pill).toHaveTextContent("云端");
      expect(screen.queryByTestId("pill-live")).toBeNull();
      // The waveform is the recorder's live level history, not the sample sequence.
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS);
      });
      expect(screen.queryByTestId("pill-waiting")).toBeNull();
      // The timer counts from the device's first samples, so two ticks of the clock read 00:01.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_METER_INTERVAL_MS * 5 + 2000);
      });
      expect(screen.getByRole("status")).toHaveTextContent("00:01");
      await act(async () => {
        await backend.invoke("dictation_stop");
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByRole("status")).toHaveTextContent("识别中…");
      expect(backend.activeMeters()).toBe(0);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByRole("status")).toHaveTextContent("润色中…");
      // docs/dictation.md §21: the tag names the preset the clean-up runs with.
      expect(screen.getByRole("status")).toHaveTextContent("校对");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS);
      });
      const done = screen.getByRole("status");
      expect(done).toHaveAttribute("data-state", "inserted");
      expect(done).toHaveTextContent(`已插入 ${MOCK_DICTATION_TEXT.length} 字`);
      expect(done).toHaveTextContent("粘贴");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByTestId("overlay-window")).toHaveAttribute("data-state", "blank");
    } finally {
      vi.useRealTimers();
    }
  });

  it("counts the step from stage_started_at, or from the run's start when a core sent none", () => {
    const processing = { phase: "processing", stage: "refining", started_at: 5 } as const;
    expect(stageStart(processing)).toBe(5);
    expect(stageStart({ ...processing, stage_started_at: 0 })).toBe(5);
    expect(stageStart({ ...processing, stage_started_at: 9 })).toBe(9);
  });

  it("regression: the processing pill counts the stage time instead of a fixed 0.0 s", async () => {
    // User feedback 2026-09-29: while transcribing and polishing the pill's timer stood at 0.0 s.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({ now: () => Date.now() });
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + 1000);
      });
      await act(async () => {
        await backend.invoke("dictation_stop");
      });
      const seconds = () => Number.parseFloat(screen.getByTestId("pill-stage-time").textContent);
      expect(screen.getByRole("status")).toHaveTextContent("识别中…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(300);
      });
      const transcribing = seconds();
      expect(transcribing).toBeGreaterThanOrEqual(0.3);
      // The next step starts its own clock.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS - 300);
      });
      expect(screen.getByRole("status")).toHaveTextContent("润色中…");
      expect(seconds()).toBeLessThan(transcribing);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(seconds()).toBeGreaterThanOrEqual(0.2);
      expect(seconds()).toBeLessThan(MOCK_ASR_MS / 1000);
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a failed run shows the reason and 复制文本 copies the kept text; cancel shows the danger pill", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const backend = new MockBackend({ now: () => Date.now() });
    renderApp({ path: "/overlay?state=live", backend });
    await screen.findByTestId("overlay-window");
    await act(async () => {
      await backend.invoke("dictation_start");
      backend.simulateDictationFailed("粘贴超时", "kept text");
    });
    const pill = await screen.findByRole("status");
    expect(pill).toHaveAttribute("data-state", "error");
    expect(pill).toHaveTextContent("未插入 · 粘贴超时");
    // The pill window has no toast viewport (chrome-less); the clipboard write is the evidence.
    await user.click(screen.getByRole("button", { name: "复制文本" }));
    expect(writeText).toHaveBeenCalledWith("kept text");
    // No text kept (silent take): there is nothing to copy, so the button is not offered at all.
    act(() => {
      backend.simulateDictationFailed("没有听到声音");
    });
    expect(screen.getByRole("status")).toHaveTextContent("未插入 · 没有听到声音");
    expect(screen.queryByRole("button", { name: "复制文本" })).toBeNull();
    expect(writeText).toHaveBeenCalledTimes(1);
    await act(async () => {
      await backend.invoke("dictation_start");
      await backend.invoke("dictation_cancel");
    });
    expect(screen.getByRole("status")).toHaveAttribute("data-state", "cancel-armed");
    expect(screen.getByRole("status")).toHaveTextContent("已取消 · 录音已丢弃");
  });

  it("regression: with live preview ready the pill shows the two-tone caption and the 预览 chip while listening, the preview in place of 转写中… while processing, and nothing extra when degraded", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({
        now: () => Date.now(),
        models: {
          [MOCK_STREAMING_MODEL_ID]: {
            kind: "installed",
            path: `~/.local/share/voltip/models/${MOCK_STREAMING_MODEL_ID}`,
            installed_at: 1_758_600_000,
          },
        },
      });
      expect(backend.peek().engines.live_preview_ready).toBe(true);
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      // Waiting for the device: no caption yet.
      expect(screen.getByTestId("pill-waiting")).toBeInTheDocument();
      expect(screen.queryByTestId("pill-live")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * 2 + 10);
      });
      // Two partials in: only the current sentence, dimmed, plus the chip; the capsule is two rows.
      const second = MOCK_LIVE_SCRIPT[1];
      if (!second) throw new Error("script");
      const pill = screen.getByRole("status");
      expect(pill).toHaveAttribute("data-state", "listening");
      expect(pill).toHaveClass("h-14");
      expect(screen.getByTestId("pill-live-committed")).toBeEmptyDOMElement();
      expect(screen.getByTestId("pill-live-current")).toHaveTextContent(second.current);
      expect(screen.getByTestId("pill-live")).toHaveTextContent("预览");
      expect(screen.queryByTestId("pill-waiting")).toBeNull();
      // The endpoint commits the first sentence; the second one grows underneath it.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_LIVE_STEP_MS * (MOCK_LIVE_SCRIPT.length - 2));
      });
      const last = MOCK_LIVE_SCRIPT.at(-1);
      const committed = last?.committed[0]?.text;
      if (!last || committed === undefined) throw new Error("script");
      // 41 characters in total: the caption is clipped to its tail with a leading ellipsis, so the
      // words being spoken stay visible.
      expect(screen.getByTestId("pill-live")).toHaveAttribute("data-clipped", "true");
      expect(screen.getByTestId("pill-live-committed")).toHaveTextContent(
        new RegExp(`^…${committed.slice(1)}$`),
      );
      expect(screen.getByTestId("pill-live-current")).toHaveTextContent(last.current);
      // A degraded preview changes nothing in the pill: the text stays, no reason is shown.
      act(() => {
        backend.simulateLiveDegraded("live tap overrun: the decoder fell behind the microphone");
      });
      expect(screen.getByRole("status")).not.toHaveTextContent(/overrun|中断|stopped/);
      expect(screen.getByTestId("pill-live-current")).toHaveTextContent(last.current);
      // Stop: the preview replaces 转写中… until the final text; the refine stage keeps it.
      await act(async () => {
        await backend.invoke("dictation_stop");
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByRole("status")).toHaveClass("h-10");
      expect(screen.getByTestId("pill-preview")).toHaveTextContent(clipTail(livePreviewText(last)));
      expect(screen.getByTestId("pill-preview")).toHaveClass("text-pill-muted");
      expect(screen.queryByText("识别中…")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByTestId("pill-preview")).toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveTextContent("校对");
      expect(screen.queryByText("润色中…")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS);
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "inserted");
      expect(screen.queryByTestId("pill-preview")).toBeNull();
      expect(screen.getByRole("status")).toHaveTextContent(
        `已插入 ${MOCK_DICTATION_TEXT.length} 字`,
      );
      // The pure mapping: no text yet → no caption; committed and current are joined per the CJK rule.
      expect(pillLiveCaption(undefined)).toBeUndefined();
      expect(pillLiveCaption({ committed: [], current: "", injected: 0 })).toBeUndefined();
      expect(
        pillLiveCaption({ committed: [], current: "", degraded: "x", injected: 0 }),
      ).toBeUndefined();
      expect(pillLiveCaption(last)).toEqual({ committed, current: last.current });
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a hold_or_toggle short press locks the take and the pill shows the lock mark; live_inject draws pasted sentences fainter, finalizing keeps the preview next to 补齐最后一句…, and a cancel after pasting says the characters stay (docs/dictation.md §12–§13)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({
        now: () => Date.now(),
        settings: {
          activation: "hold_or_toggle",
          hold_threshold_ms: 300,
          engines: { ...defaultEngineSettings(), output_mode: "live_inject" },
        },
        models: {
          [MOCK_STREAMING_MODEL_ID]: {
            kind: "installed",
            path: `~/.local/share/voltip/models/${MOCK_STREAMING_MODEL_ID}`,
            installed_at: 1_758_600_000,
          },
        },
      });
      expect(backend.peek().engines.effective_output_mode).toBe("live_inject");
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      // Short press: the take starts and locks; the pill swaps the lamp for the lock mark.
      await act(async () => {
        await backend.invoke("hotkey_edge", { pressed: true });
      });
      expect(screen.queryByTestId("pill-lock")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
        await backend.invoke("hotkey_edge", { pressed: false });
      });
      const pill = await screen.findByRole("status");
      expect(pill).toHaveAttribute("data-state", "listening");
      expect(screen.getByTestId("pill-lock")).toHaveAttribute(
        "aria-label",
        "已锁定 · 再按一次结束",
      );
      // The first endpoint commits a sentence and live_inject pastes it: the caption shows it in
      // the fainter tone, the lock stays.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(
          MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * MOCK_LIVE_SCRIPT.length + 10,
        );
      });
      const committed = MOCK_LIVE_SCRIPT.at(-1)?.committed[0]?.text;
      const current = MOCK_LIVE_SCRIPT.at(-1)?.current;
      if (committed === undefined || current === undefined) throw new Error("script");
      const phase = backend.peek().dictation.phase;
      expect(phase.phase === "listening" && phase.live?.injected).toBe(1);
      // 41 characters: the pasted part is the first to lose its head to the 40-char clip.
      expect(screen.getByTestId("pill-live")).toHaveAttribute("data-clipped", "true");
      expect(screen.getByTestId("pill-live-injected")).toHaveTextContent(
        new RegExp(`^…${committed.slice(1)}$`),
      );
      expect(screen.getByTestId("pill-live-injected")).toHaveAttribute("title", "已输入到当前窗口");
      expect(screen.getByTestId("pill-live-committed")).toBeEmptyDOMElement();
      expect(screen.getByTestId("pill-live-current")).toHaveTextContent(current);
      expect(screen.getByTestId("pill-lock")).toBeInTheDocument();
      // The next press ends the locked take: finalizing shows the stage next to the preview.
      await act(async () => {
        await backend.invoke("hotkey_edge", { pressed: true });
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByRole("status")).toHaveTextContent("补齐最后一句…");
      expect(screen.getByTestId("pill-preview")).toBeInTheDocument();
      expect(screen.queryByTestId("pill-lock")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_FINALIZE_MS * 2);
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "inserted");
      expect(screen.getByRole("status")).not.toHaveTextContent("补齐最后一句…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      // A cancel after the first paste keeps what was pasted and says so.
      await act(async () => {
        await backend.invoke("dictation_start");
        await vi.advanceTimersByTimeAsync(MOCK_MIC_READY_MS + MOCK_LIVE_STEP_MS * 4 + 10);
        await backend.invoke("dictation_cancel");
      });
      const n = Array.from(joinLiveText([committed])).length;
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "cancel-armed");
      expect(screen.getByRole("status")).toHaveTextContent(`已取消，之前打进去的 ${n} 字保留`);
      // The mapping: pasted sentences split off the committed text; none pasted → no key.
      const live = MOCK_LIVE_SCRIPT.at(-1);
      if (!live) throw new Error("script");
      expect(pillLiveCaption({ ...live, injected: 1 })).toEqual({
        injected: committed,
        committed: "",
        current: live.current,
      });
      expect(pillLiveCaption({ ...live, injected: 0 })).toEqual({
        committed,
        current: live.current,
      });
      // A locked take without a live preview still shows the lock (no streaming model needed).
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
        await backend.invoke("hotkey_edge", { pressed: true });
        await backend.invoke("hotkey_edge", { pressed: false });
      });
      expect(screen.getByTestId("pill-lock")).toBeInTheDocument();
      expect(screen.getByRole("status")).toHaveClass("h-10");
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: the live pill names the scene the take runs under next to the mode tag while listening and processing; a take without a scene shows no tag (docs/dictation.md section 18.6)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({
        now: () => Date.now(),
        scenes: [
          {
            id: "00000000-0000-4000-a000-000000000001",
            name: "聊天",
            enabled: true,
            match: { apps: ["slack"], title_contains: [] },
            overrides: { refine_enabled: false },
            created_at_ms: 1,
            updated_at_ms: 1,
          },
        ],
        foregroundApp: { id: "Slack.exe", name: "Slack", title: "#dev" },
      });
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      const listening = await screen.findByRole("status");
      expect(listening).toHaveAttribute("data-state", "listening");
      expect(screen.getByTestId("pill-scene")).toHaveTextContent("聊天");
      expect(screen.getByTestId("pill-scene")).toHaveAttribute("title", "场景：聊天");
      expect(screen.getByText("云端").nextElementSibling).toBe(screen.getByTestId("pill-scene"));
      await act(async () => {
        await backend.invoke("dictation_stop");
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByTestId("pill-scene")).toHaveTextContent("聊天");
      // The scene switched refine off: straight from recognition to the inserted pill, no LLM.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "inserted");
      expect(screen.queryByTestId("pill-scene")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByTestId("overlay-window")).toHaveAttribute("data-state", "blank");

      // Another app: the take has a context but no scene, so no tag.
      backend.setForegroundApp({ id: "code", name: "Code" });
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      expect(await screen.findByRole("status")).toHaveAttribute("data-state", "listening");
      expect(screen.queryByTestId("pill-scene")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a voice edit leads the live pill with its tag and says rewriting and replaced once pasted and localises a refusal and a matching scene never tags an edit (section 19)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const backend = new MockBackend({
        now: () => Date.now(),
        scenes: [
          {
            id: "00000000-0000-4000-a000-000000000001",
            name: "聊天",
            enabled: true,
            match: { apps: ["slack"], title_contains: [] },
            overrides: { refine_enabled: false },
            created_at_ms: 1,
            updated_at_ms: 1,
          },
        ],
        foregroundApp: { id: "Slack.exe", name: "Slack", title: "#dev" },
      });
      backend.setSelection("大家好，会议改到周四十点哈");
      renderApp({ path: "/overlay?state=live", backend });
      await screen.findByTestId("overlay-window");
      const editEdge = async (pressed: boolean) => {
        await act(async () => {
          await backend.invoke("hotkey_edge", { pressed, source: "hotkey", purpose: "edit" });
        });
      };
      await editEdge(true);
      expect(await screen.findByRole("status")).toHaveAttribute("data-state", "listening");
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("编辑");
      expect(screen.queryByTestId("pill-scene")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_COPY_MS + MOCK_MIC_READY_MS);
      });
      await editEdge(false);
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "processing");
      expect(screen.getByRole("status")).toHaveTextContent("识别中…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByRole("status")).toHaveTextContent("改写中…");
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("编辑");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS + MOCK_FINALIZE_MS);
      });
      const done = screen.getByRole("status");
      expect(done).toHaveAttribute("data-state", "inserted");
      expect(done).toHaveTextContent(`已替换 ${Array.from(MOCK_EDIT_TEXT).length} 字`);
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("编辑");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByTestId("overlay-window")).toHaveAttribute("data-state", "blank");
      // Nothing selected: the pill says why, still as an edit.
      backend.setSelection(null);
      await editEdge(true);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_COPY_MS);
      });
      expect(screen.getByRole("status")).toHaveAttribute("data-state", "error");
      expect(screen.getByRole("status")).toHaveTextContent("未插入 · 没有选中文本");
      expect(screen.getByTestId("pill-tag")).toHaveTextContent("编辑");
      await editEdge(false);
      // A dictation afterwards carries no tag.
      await act(async () => {
        await backend.invoke("dictation_start");
      });
      expect(await screen.findByRole("status")).toHaveAttribute("data-state", "listening");
      expect(screen.queryByTestId("pill-tag")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
