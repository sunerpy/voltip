import { defaultEngineSettings } from "@voltip/shared";
import {
  MOCK_ASR_MS,
  MOCK_AUDIO_DEVICES,
  MOCK_COPY_MS,
  MOCK_DICTATION_DWELL_MS,
  MOCK_EDIT_TEXT,
  MOCK_FINALIZE_MS,
  MOCK_DICTATION_TEXT,
  MOCK_MIC_READY_MS,
  MOCK_METER_INTERVAL_MS,
  MOCK_REFINE_MS,
  MOCK_STREAMING_MODEL_ID,
  MockBackend,
  phoneIdentity,
  sampleDevices,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { historyStats } from "../features/history/stats";
import { renderApp } from "../test/render";
import { MIC_TEST_MS } from "../features/audio/useMicrophoneTest";

/** Fixed pixel panel sizes and two-fixed-column grids are what broke the 1440 / 1920 px windows
 *  (Windows test 2026-09-24). `max-w-[…]` / `min-w-[…]` caps stay allowed: the page root itself is
 *  `max-w-[1600px]`. */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

/** Rows the 手机 card lists before its overflow line (Home.tsx `HOME_DEVICE_ROWS`). */
const HOME_DEVICE_ROWS = 3;

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

/** The home statistics are "today / this week" by the real clock, so the sample history must be
 *  dated relative to the same clock as the page. */
function liveClock() {
  return { now: () => Date.now() };
}

describe("Home page", () => {
  it("regression: the recent table copies and pastes a row without opening it, by mouse and by keyboard", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ mock: liveClock() });
    const table = await screen.findByRole("table", { name: "最近的结果" });
    const newest = backend.peek().history[0];
    if (!newest) throw new Error("fixture");
    const row = within(table).getAllByRole("row")[1] as HTMLElement;
    // Copy: the row's text, the usual toast, and the page stays.
    await user.click(within(row).getByRole("button", { name: "复制这条结果" }));
    expect(writeText).toHaveBeenCalledWith(newest.text);
    expect(
      await screen.findByText(`已复制到剪贴板 · ${Array.from(newest.text).length} 字`),
    ).toBeInTheDocument();
    expect(screen.getByTestId("page-home")).toBeInTheDocument();
    // Paste into the previous window: the text goes to paste_text, the answer becomes a toast.
    await user.click(within(row).getByRole("button", { name: "粘贴到上一个窗口" }));
    expect(backend.pastes).toEqual([newest.text]);
    expect(await screen.findByText("已粘贴到上一个窗口")).toBeInTheDocument();
    expect(screen.getByTestId("page-home")).toBeInTheDocument();
    // The keyboard reaches the button, not the row; a copy instead says why.
    backend.setPasteOutcome({ kind: "copied", reason: "target_changed" });
    within(row).getByRole("button", { name: "粘贴到上一个窗口" }).focus();
    await user.keyboard("{Enter}");
    expect(await screen.findByText("前台窗口已变化 · 仅复制到剪贴板")).toBeInTheDocument();
    expect(backend.pastes).toHaveLength(2);
    expect(screen.getByTestId("page-home")).toBeInTheDocument();
    // The row itself still opens the entry.
    await user.click(within(row).getByText(newest.text));
    expect(await screen.findByTestId("page-history")).toBeInTheDocument();
  });

  it("a paste refused while a take runs says so, and a voice edit's row hands on its rewrite", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const now = Date.now();
    const base = {
      at_ms: now - 60_000,
      asr_model: "whisper-large-v3-turbo",
      duration_ms: 1400,
      asr_ms: 380,
      refined: true,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
    } as const;
    const { backend } = renderApp({
      backend: new MockBackend({
        now: () => Date.now(),
        history: [
          {
            ...base,
            id: "edit",
            raw_text: "改得更正式",
            text: "各位同事：会议改至周四上午十点。",
            kind: "edit",
            edit: { instruction: "改得更正式", selection: "大家好，会议改到周四十点哈" },
          },
        ],
      }),
    });
    const table = await screen.findByRole("table", { name: "最近的结果" });
    await user.click(within(table).getByRole("button", { name: "复制这条结果" }));
    expect(writeText).toHaveBeenCalledWith("各位同事：会议改至周四上午十点。");
    await act(async () => {
      await backend.invoke("dictation_start");
    });
    await user.click(within(table).getByRole("button", { name: "粘贴到上一个窗口" }));
    expect(await screen.findByText("正在听写 · 请结束后再粘贴")).toBeInTheDocument();
    expect(backend.pastes).toEqual([]);
  });

  it("renders readiness row, four panels, stat strip and the recent table from the core's state", async () => {
    const { backend } = renderApp({ mock: liveClock() });
    expect(await screen.findByText("可以开始听写")).toBeInTheDocument();
    // The strength bar exists but stays silent while idle (the regression test below drives it).
    expect(screen.getByRole("meter", { name: "强度" })).toBeInTheDocument();
    expect(screen.getByTestId("home-phase")).toHaveTextContent(
      "麦克风、快捷键和识别服务已就绪 · Qwen3-ASR-1.7B · 内置服务",
    );
    expect(screen.getByText("麦克风输入")).toBeInTheDocument();
    expect(screen.getByText("Fifine K669 USB Microphone")).toBeInTheDocument();
    expect(within(screen.getByTestId("home-engine")).getByText("语音模型")).toBeInTheDocument();
    expect(within(screen.getByTestId("home-devices")).getByText("手机 · 设备")).toBeInTheDocument();
    expect(screen.getByText("今日听写")).toBeInTheDocument();
    // The engine card reads state.engines, not a fixture.
    const engine = screen.getByTestId("home-engine");
    expect(within(engine).getByText("Qwen3-ASR-1.7B")).toBeInTheDocument();
    expect(within(engine).getByText(/内置服务 · 语言 自动/)).toBeInTheDocument();
    expect(within(engine).getByText("qwen3.8-27b")).toBeInTheDocument();
    expect(within(engine).getByRole("switch", { name: /AI 润色 开/ })).toBeChecked();
    expect(screen.getByTestId("home-privacy")).toHaveTextContent(
      "音频发送到内置服务 · 文本发送到内置服务",
    );
    // Regression (public release, 2026-09-27): the home page never names the built-in host.
    expect(screen.getByTestId("page-home").textContent).not.toMatch(/voltip\.example|groq\.com/);
    expect(
      [...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title")).join(" "),
    ).not.toMatch(/voltip\.example/);

    // Stats and tiles come from state.history (the sample rows dated relative to now).
    const stats = historyStats(backend.peek().history, Date.now());
    expect(stats.today.count).toBe(2);
    expect(screen.getByRole("button", { name: "今天 2 条" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `本周 ${stats.week.count} 条` })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `本月 ${stats.month.count} 条` }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "总计 6 / 500" })).toBeInTheDocument();
    const session = screen.getByTestId("home-session");
    expect(within(session).getByText("2")).toBeInTheDocument();
    expect(within(session).getByText(String(stats.today.latencyMs))).toBeInTheDocument();
    const table = screen.getByRole("table", { name: "最近的结果" });
    expect(within(table).getAllByRole("row")).toHaveLength(6 + 1);
    expect(within(table).getAllByText("Qwen3-ASR-1.7B").length).toBe(6);
    expect(within(table).getByText(/attach the latency report/)).toBeInTheDocument();
    expect(within(table).getByText("仅剪贴板 · 目标窗口没有焦点")).toBeInTheDocument();
    expect(within(table).getByText("失败 · 目标窗口已丢失")).toBeInTheDocument();
    // Nothing on the page talks about phases or sample data any more.
    expect(screen.getByTestId("page-home").textContent).not.toMatch(/第二阶段|示例数据/);
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
  });

  it("regression: the microphone stays closed while idle; 测试麦克风 meters it for 15 s, a take meters its own recording (user feedback 2026-09-28)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend, unmount } = renderApp();
      expect(await screen.findByText("可以开始听写")).toBeInTheDocument();
      await waitFor(() => {
        expect(screen.getByTestId("home-mic-device")).toHaveTextContent(
          MOCK_AUDIO_DEVICES[0]?.name ?? "",
        );
      });
      // Idle: nothing opens the microphone, the bar is silent and says why.
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 5);
      });
      expect(backend.activeMeters()).toBe(0);
      expect(screen.getByTestId("home-mic-state")).toHaveTextContent("空闲 · 未打开麦克风");
      expect(screen.getByTestId("home-mic-level")).toHaveTextContent("— dBFS");
      expect(screen.getByTestId("home-mic-hint")).toHaveTextContent("空闲时不打开麦克风");
      expect(screen.getByText("48 kHz · 单声道 · 系统默认")).toBeInTheDocument();
      // No 电平 anywhere: the bar is 强度.
      expect(screen.getByTestId("page-home").textContent).not.toMatch(/电平/);
      // 测试麦克风: the meter runs, the bar moves, and it closes by itself after 15 s.
      await user.click(screen.getByTestId("home-mic-test"));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 2 + 1);
      });
      const level = screen.getByTestId("home-mic-level");
      expect(level).toHaveTextContent(/-\d+\.\d dBFS/);
      expect(level).toHaveTextContent(/峰值 -\d+\.\d/);
      const meter = screen.getByRole("meter", { name: "强度" });
      expect(Number(meter.getAttribute("aria-valuenow"))).toBeGreaterThan(0);
      expect(screen.getByTestId("home-mic-state")).toHaveTextContent(/测试中 · 1[45] 秒/);
      expect(screen.getByTestId("home-mic-test")).toHaveTextContent("停止测试");
      act(() => {
        vi.advanceTimersByTime(MIC_TEST_MS + 500);
      });
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(0);
      });
      expect(screen.getByTestId("home-mic-state")).toHaveTextContent("空闲 · 未打开麦克风");
      // 停止测试 ends a run early.
      await user.click(screen.getByTestId("home-mic-test"));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      await user.click(screen.getByTestId("home-mic-test"));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(0);
      });
      // A take meters its own recording, and the meter closes with it.
      await user.click(screen.getByRole("button", { name: "开始听写" }));
      await waitFor(() => {
        expect(screen.getByTestId("home-mic-state")).toHaveTextContent("录音中");
      });
      expect(backend.activeMeters()).toBe(1);
      expect(screen.queryByTestId("home-mic-test")).toBeNull();
      await user.click(screen.getByRole("button", { name: "取消" }));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(0);
      });
      unmount();
      expect(backend.activeMeters()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: 切换麦克风 opens 设置 › 麦克风, and an unplugged choice is named as such", async () => {
    const user = userEvent.setup();
    renderApp({ mock: { settings: { microphone: "Blue Yeti" } } });
    expect(await screen.findByTestId("home-mic-missing")).toHaveTextContent(
      "所选麦克风未连接 · 使用系统默认",
    );
    expect(screen.getByTestId("home-mic-device")).toHaveTextContent(
      MOCK_AUDIO_DEVICES[0]?.name ?? "",
    );
    await user.click(screen.getByTestId("home-mic-switch"));
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("tab", { name: "麦克风", selected: true })).toBeInTheDocument();
    expect(within(dialog).getByTestId("microphone-pane")).toBeInTheDocument();
  });

  it("regression: 开始听写 starts a real session and shows the phase; 停止 finishes with the inserted text", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({ mock: liveClock() });
      await screen.findByText("可以开始听写");
      const before = backend.peek().history.length;
      const start = screen.getByRole("button", { name: "开始听写" });
      expect(start).toBeEnabled();
      expect(start).not.toHaveAttribute("title");
      await user.click(start);
      expect(backend.peek().dictation.phase.phase).toBe("listening");
      expect(screen.getByText("正在听写")).toBeInTheDocument();
      expect(screen.getByTestId("home-phase")).toHaveTextContent(/正在录音… 00:0\d/);
      expect(screen.queryByRole("button", { name: "开始听写" })).toBeNull();
      // The live meter names the recording.
      await waitFor(() => {
        expect(within(screen.getByTestId("home-mic")).getByText("录音中")).toBeInTheDocument();
      });
      // The timer counts from the device's first samples (150 ms after start), not from the click:
      // three ticks of the one-second clock read 00:02.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000 + MOCK_MIC_READY_MS);
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent(/正在录音… 00:02/);
      await user.click(screen.getByRole("button", { name: "停止" }));
      expect(backend.peek().dictation.phase).toMatchObject({
        phase: "processing",
        stage: "transcribing",
      });
      expect(screen.getByText("正在处理")).toBeInTheDocument();
      expect(screen.getByTestId("home-phase")).toHaveTextContent("识别中…");
      expect(screen.getByRole("button", { name: "处理中…" })).toBeDisabled();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("润色中…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS);
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent(
        `已插入 ${MOCK_DICTATION_TEXT.length} 字 · 粘贴 · 已润色`,
      );
      // The result landed in history: the table, the tiles and the sidebar count all moved.
      expect(backend.peek().history).toHaveLength(before + 1);
      const table = screen.getByRole("table", { name: "最近的结果" });
      expect(within(table).getAllByRole("row")[1]).toHaveTextContent(MOCK_DICTATION_TEXT);
      expect(screen.getByRole("button", { name: "今天 3 条" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: `总计 ${before + 1} / 500` })).toBeInTheDocument();
      expect(
        within(screen.getByRole("navigation")).getByText(String(before + 1)),
      ).toBeInTheDocument();
      // The core returns to idle after its dwell; the button is 开始听写 again.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByText("可以开始听写")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "开始听写" })).toBeEnabled();
      expect(screen.queryByText(/已开始听写/)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a voice edit reads as one on the home page while listening and rewriting and once replaced and its row is badged in the recent table (section 19)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { backend } = renderApp({ mock: liveClock() });
      await screen.findByText("可以开始听写");
      backend.setSelection("大家好，会议改到周四十点哈");
      const editEdge = async (pressed: boolean) => {
        await act(async () => {
          await backend.invoke("hotkey_edge", { pressed, source: "hotkey", purpose: "edit" });
        });
      };
      await editEdge(true);
      expect(screen.getByTestId("home-phase")).toHaveTextContent(/正在听编辑指令… 00:0\d/);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_COPY_MS + MOCK_MIC_READY_MS + 1000);
      });
      await editEdge(false);
      expect(screen.getByTestId("home-phase")).toHaveTextContent("识别中…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_ASR_MS);
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("改写中…");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_REFINE_MS + MOCK_FINALIZE_MS);
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent(
        `已替换 ${Array.from(MOCK_EDIT_TEXT).length} 字 · 粘贴`,
      );
      const table = screen.getByRole("table", { name: "最近的结果" });
      const newest = within(table).getAllByRole("row")[1] as HTMLElement;
      expect(newest).toHaveTextContent(MOCK_EDIT_TEXT);
      expect(within(newest).getByTestId("home-edit-badge")).toHaveTextContent("编辑");
      expect(within(table).getAllByTestId("home-edit-badge")).toHaveLength(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      expect(screen.getByText("可以开始听写")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("取消 during listening discards the recording; a failed run shows the reason", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({ mock: liveClock() });
      await screen.findByText("可以开始听写");
      const before = backend.peek().history.length;
      await user.click(screen.getByRole("button", { name: "开始听写" }));
      await user.click(screen.getByRole("button", { name: "取消" }));
      expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("已取消");
      expect(backend.peek().history).toHaveLength(before);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_DICTATION_DWELL_MS);
      });
      await user.click(screen.getByRole("button", { name: "开始听写" }));
      act(() => {
        backend.simulateDictationFailed("没有听到声音");
      });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("失败 · 没有听到声音");
      expect(screen.getByTestId("home-phase")).toHaveClass("text-danger");
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: the engine card shows the 实时预览 chip when live_preview_ready and a one-line note when the live preview degraded; the local model name follows the core", async () => {
    const backend = new MockBackend({
      now: () => Date.now(),
      models: {
        [MOCK_STREAMING_MODEL_ID]: {
          kind: "installed",
          path: `~/.local/share/voltip/models/${MOCK_STREAMING_MODEL_ID}`,
          installed_at: 1_758_600_000,
        },
        "qwen3-asr-0.6b": {
          kind: "installed",
          path: "~/.local/share/voltip/models/qwen3-asr-0.6b",
          installed_at: 1_758_600_000,
        },
      },
    });
    renderApp({ backend });
    const engine = await screen.findByTestId("home-engine");
    expect(backend.peek().engines.live_preview_ready).toBe(true);
    expect(within(engine).getByTestId("home-live-preview")).toHaveTextContent("实时预览");
    expect(screen.queryByTestId("home-live-degraded")).toBeNull();
    // The note appears only while the take is listening with a degraded preview, and names no
    // error: the final text is unaffected.
    await act(async () => {
      await backend.invoke("dictation_start");
    });
    expect(screen.queryByTestId("home-live-degraded")).toBeNull();
    act(() => {
      backend.simulateLiveDegraded("live tap overrun: the decoder fell behind the microphone");
    });
    const note = within(engine).getByTestId("home-live-degraded");
    expect(note).toHaveTextContent("实时预览已中断 · 最终文本不受影响");
    expect(note).toHaveAttribute(
      "title",
      "live tap overrun: the decoder fell behind the microphone",
    );
    expect(screen.queryByText("正在处理")).toBeNull();
    await act(async () => {
      await backend.invoke("dictation_cancel");
    });
    expect(screen.queryByTestId("home-live-degraded")).toBeNull();
    // Switching live preview off drops the chip; local mode shows the core's tier name.
    await act(async () => {
      await backend.invoke("settings_set_engines", {
        engines: { ...backend.peek().settings.engines, asr_provider: "local", live_preview: false },
      });
    });
    expect(within(engine).queryByTestId("home-live-preview")).toBeNull();
    expect(within(engine).getByText("均衡")).toBeInTheDocument();
    expect(within(engine).getByText("本地")).toBeInTheDocument();
    // The readiness chip names the tier too (the phase line is still in the cancel dwell here).
    expect(screen.getByText("本机 · 均衡")).toBeInTheDocument();
  });

  it("regression: changing the hotkey updates the home readiness row and footer immediately", async () => {
    const { backend } = renderApp();
    const row = await screen.findByTestId("home-readiness");
    expect(within(row).getByLabelText("Ctrl Alt Space")).toBeInTheDocument();
    const footer = screen.getByText("按住听写").closest("footer") as HTMLElement;
    expect(within(footer).getByLabelText("Ctrl Alt Space")).toBeInTheDocument();
    await act(async () => {
      await backend.invoke("settings_set_hotkey", { hotkey: "Ctrl+Shift+D" });
    });
    expect(within(row).getByLabelText("Ctrl Shift D")).toBeInTheDocument();
    expect(within(row).queryByLabelText("Ctrl Alt Space")).toBeNull();
    expect(within(footer).getByLabelText("Ctrl Shift D")).toBeInTheDocument();
    expect(within(footer).queryByLabelText("Ctrl Alt Space")).toBeNull();
    // Nothing on the page still shows the fixture chord.
    expect(screen.getByTestId("page-home").textContent).not.toContain("Ctrl Alt Space");
  });

  it("blocks dictation with a plain reason when the recognition provider has no key, and shows the empty state without history", async () => {
    renderApp({
      backend: new MockBackend({
        history: [],
        settings: {
          hotkey: "Ctrl+Alt+Space",
          engines: { ...defaultEngineSettings(), asr_provider: "groq" },
        },
      }),
    });
    expect(await screen.findByText("还不能开始听写")).toBeInTheDocument();
    const start = screen.getByRole("button", { name: "开始听写" });
    expect(start).toBeDisabled();
    // The reason follows `state.engines`, which the backend reports asynchronously after the first
    // paint (until then the button says the core is still being awaited): wait for it explicitly.
    await waitFor(() => {
      expect(start).toHaveAttribute(
        "title",
        "语音识别服务商不可用：缺少密钥 · 请在「语音模型」页配置",
      );
    });
    expect(screen.getByTestId("home-phase")).toHaveTextContent("缺少密钥");
    expect(screen.getByRole("button", { name: "今天 0 条" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "总计 0 / 500" })).toBeInTheDocument();
    expect(screen.queryByRole("table", { name: "最近的结果" })).toBeNull();
    expect(screen.getByText("暂无听写结果")).toBeInTheDocument();
    expect(screen.getByText("按住 Ctrl Alt Space 说一句，松开即插入")).toBeInTheDocument();
    // The microphone card shows its own `—` until the native enumeration lands; wait for the
    // device so the only remaining `—` is the average-latency readout.
    expect(await screen.findByText("Fifine K669")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("regression: the readiness chip and the empty-state hint follow settings.activation (docs/dictation.md §13)", async () => {
    const { backend } = renderApp({
      mock: { history: [], settings: { activation: "toggle" } },
    });
    expect(
      await screen.findByRole("button", { name: "按一下开始 · 再按结束" }),
    ).toBeInTheDocument();
    expect(screen.getByText("按一下 Ctrl Alt Space 开始，再按一下结束")).toBeInTheDocument();
    expect(screen.getByText("按一下听写")).toBeInTheDocument();
    expect(screen.queryByText("按住说话")).toBeNull();
    // The core switches the mode (settings event): chip, hint and footer follow at once.
    act(() => {
      backend.publish({
        type: "settings",
        ...backend.peek().settings,
        activation: "hold_or_toggle",
      });
    });
    expect(await screen.findByRole("button", { name: "按住说话 · 短按锁定" })).toBeInTheDocument();
    expect(screen.getByText("按住 Ctrl Alt Space 说话，短按锁定")).toBeInTheDocument();
    expect(screen.getByText("按住或按一下听写")).toBeInTheDocument();
    act(() => {
      backend.publish({ type: "settings", ...backend.peek().settings, activation: "hold" });
    });
    expect(await screen.findByRole("button", { name: "按住说话" })).toBeInTheDocument();
    expect(screen.getByText("按住 Ctrl Alt Space 说一句，松开即插入")).toBeInTheDocument();
    expect(screen.getByText("按住听写")).toBeInTheDocument();
  });

  it("regression: the Bridge & MCP card is gone; the 手机 card reads the core's devices and relay", async () => {
    const user = userEvent.setup();
    renderApp();
    const page = await screen.findByTestId("page-home");
    // Windows test 2026-09-25: Bridge & MCP is being removed, nothing on the page may mention it.
    expect(page.textContent).not.toMatch(/Bridge|MCP|Claude Code|OpenCode|Hook/);
    expect(
      screen.queryByRole("button", { name: /复制 MCP 配置|复制 Hook 命令|待批准/ }),
    ).toBeNull();
    const card = screen.getByTestId("home-devices");
    // renderApp's core: Pixel 8 online over LAN, MacBook Pro offline, relay not configured.
    // Header lamp (first online phone) and the Pixel 8 row both read the core's connection label.
    expect(within(card).getAllByText("在线 · 直连")).toHaveLength(2);
    expect(within(card).getByText("2 台已配对")).toBeInTheDocument();
    const rows = within(card).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      "Pixel 8Android在线 · 直连",
      "MacBook PromacOS离线",
    ]);
    expect(within(card).getByText("中继 · 未配置")).toBeInTheDocument();
    expect(within(card).queryByText("尚未配对手机 · 去配对")).toBeNull();
    await user.click(within(card).getByRole("button", { name: "打开「手机」页" }));
    expect(screen.getByRole("heading", { name: "手机", level: 1 })).toBeInTheDocument();
  });

  it("regression: with no paired phone the 手机 card shows the inline 去配对 hint and it navigates", async () => {
    const user = userEvent.setup();
    const many = sampleDevices(1_758_700_000);
    renderApp({ backend: new MockBackend({ devices: [] }) });
    const card = await screen.findByTestId("home-devices");
    expect(within(card).getByText("未配对")).toBeInTheDocument();
    expect(within(card).getByText("0 台已配对")).toBeInTheDocument();
    expect(within(card).queryByRole("listitem")).toBeNull();
    await user.click(within(card).getByRole("button", { name: "尚未配对手机 · 去配对" }));
    expect(screen.getByRole("heading", { name: "手机", level: 1 })).toBeInTheDocument();
    // More phones than the card lists: the overflow line points at the devices page.
    const extra = many.map((d, i) => ({
      ...d,
      device: { ...d.device, public_key: `${d.device.public_key.slice(0, -1)}${i}` },
    }));
    renderApp({
      backend: new MockBackend({
        devices: [...many, ...extra].map((d) => ({ ...d, connection: { state: "connecting" } })),
        relay: {
          state: "reconnecting",
          attempts: 2,
          endpoint: "wss://relay.example.test",
          source: "user",
        },
      }),
    });
    const crowded = (await screen.findAllByTestId("home-devices")).at(-1) as HTMLElement;
    expect(within(crowded).getAllByText("连接中").length).toBeGreaterThan(HOME_DEVICE_ROWS);
    expect(within(crowded).getByText("还有 1 台 · 在「手机」页查看")).toBeInTheDocument();
    expect(within(crowded).getByText("中继 · 重连中 · 第 2 次")).toBeInTheDocument();
    // Paired but nothing online or connecting: the header lamp reads 离线.
    renderApp({
      backend: new MockBackend({
        devices: many.map((d) => ({ ...d, connection: { state: "offline" } })),
      }),
    });
    const offline = (await screen.findAllByTestId("home-devices")).at(-1) as HTMLElement;
    expect(within(offline).getAllByText("离线")).toHaveLength(many.length + 1);
  });

  it("regression: home is fluid (no fixed-width panels)", async () => {
    renderApp();
    const page = await screen.findByTestId("page-home");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1600px]", "p-6");
    // Allow-list: none on this board (keycaps and the LED meter size themselves with spacing
    // utilities, the heatmap with inline pixel cells).
    expect(fixedSizeOffenders(page)).toEqual([]);
    for (const grid of page.querySelectorAll<HTMLElement>(".grid"))
      expect(grid.className).not.toMatch(/(?:^|\s)grid-cols-\[[^\]]*\d+px/);
  });

  it("the AI 润色 toggle writes settings_set_engines and the card follows the core", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByText("可以开始听写");
    await user.click(screen.getByRole("switch", { name: /AI 润色 开/ }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(backend.peek().engines.refine_enabled).toBe(false);
    expect(screen.getByRole("switch", { name: "AI 润色 关" })).not.toBeChecked();
    expect(screen.getByTestId("home-privacy")).toHaveTextContent("音频发送到内置服务");
    expect(screen.getByTestId("home-privacy")).not.toHaveTextContent("文本发送到");
    expect(within(screen.getByTestId("home-engine")).getByText("关")).toBeInTheDocument();
    // 配置语音模型 opens the 语音模型 page (a page of the main layout since 2026-09-28).
    await user.click(screen.getByRole("button", { name: "配置语音模型" }));
    expect(await screen.findByTestId("page-speech")).toBeInTheDocument();
    expect(screen.getByTestId("speech-pane")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    await user.click(screen.getByRole("button", { name: /^首页$/ }));
    await user.click(screen.getByRole("button", { name: "查看全部 →" }));
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
  });

  it("stat tiles, chips and recent rows navigate on every platform", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      backend: new MockBackend({
        identity: { ...phoneIdentity(), platform: "macos" },
        now: () => Date.now(),
      }),
    });
    await screen.findByText("可以开始听写");
    expect(screen.getByRole("button", { name: "开始听写" })).toBeEnabled();
    const stats = historyStats(backend.peek().history, Date.now());
    await user.click(screen.getByRole("button", { name: `本周 ${stats.week.count} 条` }));
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /首页/ }));
    // Settings open as a modal over the page (Shell); 按住说话 lands on the 热键 group.
    await user.click(screen.getByRole("button", { name: "按住说话" }));
    const settings = screen.getByRole("dialog", { name: "设置" });
    expect(within(settings).getByRole("tab", { name: /快捷键/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "内置服务 · Qwen3-ASR-1.7B" }));
    expect(await screen.findByTestId("page-speech")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^首页$/ }));
    const tile = screen.getByRole("button", { name: `本月 ${stats.month.count} 条` });
    tile.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /首页/ }));
    await user.click(
      within(screen.getByRole("table", { name: "最近的结果" })).getByText(/latency report/),
    );
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    // The recent row's id pre-selects it in the history detail.
    expect(screen.getByTestId("entry-text")).toHaveTextContent(/attach the latency report/);
  });
});
