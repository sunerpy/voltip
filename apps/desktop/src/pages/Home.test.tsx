import {
  type HistoryEntry,
  defaultEngineSettings,
  formatCount,
  formatDuration,
} from "@voltip/shared";
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
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { homeStats, statsBoundaries } from "../features/history/stats";
import { renderApp } from "../test/render";
import { MIC_TEST_MS } from "../features/audio/useMicrophoneTest";
import { appHue } from "./Home";

/** Fixed pixel panel sizes and two-fixed-column grids are what broke the 1440 / 1920 px windows
 *  (Windows test 2026-09-24). `max-w-[…]` / `min-w-[…]` caps stay allowed: the page root itself is
 *  `max-w-[1600px]`. */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

/** The recent results' layout, which jsdom does not do: one row 60 px, a day heading 28 px, the
 *  rest of the page 500 px. */
const RECENT_ROW = 60;
const RECENT_HEADING = 28;
const RECENT_REST = 500;

/** Stubs the scroll area's height and the page's boxes; `resize` sets the area and measures. */
function recentLayout() {
  let area = 0;
  const observers = new Set<() => void>();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private readonly callback: () => void) {}
      observe() {
        observers.add(this.callback);
      }
      unobserve() {}
      disconnect() {
        observers.delete(this.callback);
      }
    },
  );
  const list = () => {
    const box = document.querySelector('[data-testid="home-recent-list"]');
    if (box === null) return 0;
    return (
      box.querySelectorAll("[data-recent-row]").length * RECENT_ROW +
      box.querySelectorAll("[data-recent-day]").length * RECENT_HEADING
    );
  };
  const clientHeight = vi
    .spyOn(Element.prototype, "clientHeight", "get")
    .mockImplementation(function (this: Element) {
      return this.querySelector(':scope > [data-testid="page-home"]') === null ? 0 : area;
    });
  const rect = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (
    this: Element,
  ) {
    const el = this as HTMLElement;
    const height =
      el.dataset.testid === "page-home"
        ? RECENT_REST + list()
        : el.dataset.testid === "home-recent-list"
          ? list()
          : el.hasAttribute("data-recent-row")
            ? RECENT_ROW
            : el.hasAttribute("data-recent-day")
              ? RECENT_HEADING
              : 0;
    return new DOMRect(0, 0, 1000, height);
  });
  return {
    resize(height: number) {
      area = height;
      act(() => {
        for (const measure of observers) measure();
      });
    },
    restore() {
      clientHeight.mockRestore();
      rect.mockRestore();
      vi.unstubAllGlobals();
    },
  };
}

/** `count` dictations a minute apart, the newest at `at`; ids from `from`. */
function takes(at: number, count: number, from = 0): HistoryEntry[] {
  return Array.from({ length: count }, (_, i) => ({
    id: `00000000-0000-4000-8000-${String(from + i).padStart(12, "0")}`,
    at_ms: at - i * 60_000,
    raw_text: `第${from + i}句`,
    text: `第${from + i}句。`,
    refined: false,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 1000,
    asr_ms: 100,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
  }));
}

/** The home statistics are "today / this week" by the real clock, so the sample history must be
 *  dated relative to the same clock as the page. */
function liveClock() {
  return { now: () => Date.now() };
}

/** What the home page shows for the backend's history: the core's answer, turned into tiles. */
async function homeStatsOf(backend: MockBackend) {
  const boundaries = statsBoundaries(Date.now());
  return homeStats(await backend.historyStats(boundaries), boundaries, Date.now());
}

/** The recent results card and its rows, newest first. */
async function recentRows(): Promise<HTMLElement[]> {
  const card = await screen.findByTestId("home-recent");
  return [...card.querySelectorAll<HTMLElement>("[data-recent-row]")];
}

function firstRow(rows: readonly HTMLElement[]): HTMLElement {
  const row = rows[0];
  if (row === undefined) throw new Error("no recent row");
  return row;
}

describe("Home page", () => {
  it("regression: a recent row copies and pastes without opening it, by mouse and by keyboard", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { backend } = renderApp({ mock: liveClock() });
    const newest = backend.peek().history_recent[0];
    if (!newest) throw new Error("fixture");
    const row = firstRow(await recentRows());
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
    await user.click(within(row).getByTestId("home-row-open"));
    expect(await screen.findByTestId("page-history")).toBeInTheDocument();
  });

  it("a recent row's actions come up with the pointer or the keyboard focus, and 删除 asks first", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ mock: liveClock() });
    const before = backend.peek().history_recent.length;
    const row = firstRow(await recentRows());
    // Board 01: hidden at rest, shown on hover and on focus within the row.
    expect(within(row).getByTestId("home-row-actions")).toHaveClass(
      "opacity-0",
      "group-hover:opacity-100",
      "group-focus-within:opacity-100",
    );
    await user.click(within(row).getByRole("button", { name: "删除" }));
    const dialog = await screen.findByRole("dialog", { name: "删除这条记录？" });
    await user.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().history_recent).toHaveLength(before - 1);
    });
    expect(screen.getByTestId("page-home")).toBeInTheDocument();
  });

  // User request 2026-10-01: on a tall window the rows left a large blank space under the list.
  it("regression: the recent results fill a tall window with more rows and give them back when the window shrinks", async () => {
    const layout = recentLayout();
    try {
      const now = new Date(2026, 9, 11, 12).getTime();
      renderApp({ mock: { now: () => now, history: takes(now, 30) } });
      const card = await screen.findByTestId("home-recent");
      const rows = () => card.querySelectorAll("[data-recent-row]").length;
      // One day: its heading, then as many 60 px rows as the rest of the window holds.
      layout.resize(841);
      expect(rows()).toBe(5);
      layout.resize(1021);
      expect(rows()).toBe(8);
      // Taller than thirty rows need: all thirty, the most the core sends.
      layout.resize(2600);
      expect(rows()).toBe(30);
      // Back to a short window: the four rows of the board, the page scrolls as before.
      layout.resize(600);
      expect(rows()).toBe(4);
    } finally {
      layout.restore();
    }
  });

  // User report 2026-10-11: a maximized window flickered for ever. With room for one row more, the
  // 17th row came in and opened 昨天; its heading took the page past the bottom, the row went, the
  // heading with it, and there was room again.
  it("regression: at a day boundary the recent results settle on one count instead of flickering", async () => {
    const layout = recentLayout();
    try {
      const now = new Date(2026, 9, 11, 12).getTime();
      const history = [...takes(now, 16), ...takes(now - 24 * 3_600_000, 14, 16)];
      renderApp({ mock: { now: () => now, history } });
      const card = await screen.findByTestId("home-recent");
      const rows = () => card.querySelectorAll("[data-recent-row]").length;
      // Room for the heading, sixteen rows and 70 px: the 17th row would fit, 昨天 above it not.
      const area = RECENT_REST + 1 + RECENT_HEADING + 16 * RECENT_ROW + 70;
      const seen = new Set<number>();
      for (let i = 0; i < 6; i++) {
        // The browser measures again after every change of the page's height.
        layout.resize(area);
        seen.add(rows());
      }
      expect([...seen]).toEqual([16]);
      expect(card.querySelectorAll("[data-recent-day]")).toHaveLength(1);
      // With the heading's room too, the 17th row comes in under 昨天, and stays.
      seen.clear();
      for (let i = 0; i < 6; i++) {
        layout.resize(area + RECENT_HEADING);
        seen.add(rows());
      }
      expect([...seen]).toEqual([17]);
      expect(card.querySelectorAll("[data-recent-day]")).toHaveLength(2);
    } finally {
      layout.restore();
    }
  });

  // The real window under Xvfb (2026-10-01) showed 润色模型「关」 beside a title bar naming the
  // model, and the button's name (AI 润色模型：qwen3.8-27b) did not contain what it showed.
  it("regression: the engine card names the polish model while 润色 is off, as the title bar does, and its button is named by what it shows", async () => {
    renderApp({
      mock: { settings: { engines: { ...defaultEngineSettings(), refine_enabled: false } } },
    });
    const model = await screen.findByTestId("home-refine-model");
    expect(model).toHaveTextContent("qwen3.8-27b");
    expect(model).toHaveAccessibleName("AI 润色模型：qwen3.8-27b");
    expect(screen.getByTestId("polish-model")).toHaveTextContent("qwen3.8-27b");
  });

  it("docs/dictation.md section 3.6: the built-in AI polish service's notice leads to AI 模型 and closes", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ mock: liveClock() });
    await screen.findByTestId("home-engine");
    expect(screen.queryByTestId("refine-notice")).toBeNull();
    act(() => {
      backend.simulateRefineNotice("rate_limited");
    });
    expect(await screen.findByText("内置 AI 润色服务当前繁忙")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "关闭" }));
    await waitFor(() => {
      expect(screen.queryByTestId("refine-notice")).toBeNull();
    });
    act(() => {
      backend.simulateRefineNotice("quota");
    });
    expect(await screen.findByText("内置 AI 润色服务的额度已用完")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "打开 AI 模型" }));
    expect(await screen.findByTestId("page-ai")).toBeInTheDocument();
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
    const row = firstRow(await recentRows());
    await user.click(within(row).getByRole("button", { name: "复制这条结果" }));
    expect(writeText).toHaveBeenCalledWith("各位同事：会议改至周四上午十点。");
    await act(async () => {
      await backend.invoke("dictation_start");
    });
    await user.click(within(row).getByRole("button", { name: "粘贴到上一个窗口" }));
    expect(await screen.findByText("正在听写 · 请结束后再粘贴")).toBeInTheDocument();
    expect(backend.pastes).toEqual([]);
  });

  it("regression: the ready line says what is ready and reads whole on hover; the model is the engine card's", async () => {
    // The 1280 px English check (plan 1.2) found 「Microphone, shortcut and recognition ready ·
    // Qwen3-ASR-1.7B · Built-in…」 cut: the line names no model, the 识别与润色 card does.
    renderApp({ mock: liveClock() });
    const phase = await screen.findByTestId("home-phase");
    expect(phase).toHaveTextContent(/^麦克风、快捷键和识别服务已就绪$/);
    expect(phase).toHaveAttribute("title", "麦克风、快捷键和识别服务已就绪");
    expect(phase).toHaveClass("truncate");
    expect(screen.getByTestId("home-readiness")).toHaveClass("min-w-0");
    const engine = screen.getByTestId("home-engine");
    expect(within(engine).getByTestId("home-engine-model")).toHaveTextContent("Qwen3-ASR-1.7B");
    expect(within(engine).getByTestId("home-engine-detail")).toHaveTextContent(
      "内置服务 · 语言 自动",
    );
  });

  it("regression: the line under the dictation button wraps instead of being cut, and reads whole on hover", async () => {
    // The 1440 px English capture (2026-10-11) cut 「Idle · microphone closed」 to 「Idle · microphone clos…」.
    renderApp({ mock: { settings: { locale: "en" } } });
    const line = await screen.findByTestId("home-mic-state");
    expect(line).toHaveTextContent("Idle · microphone closed");
    expect(line).toHaveAttribute("title", "Idle · microphone closed");
    expect(line).toHaveClass("line-clamp-2", "text-center");
    expect(line).not.toHaveClass("truncate");
  });

  it("regression: a stat tile's note reads whole on hover when the default window cuts it", async () => {
    // The 1152 px check (the default window, plan 1.2): 「Keeps the newest 500」 was cut with no way
    // to read it.
    renderApp({ mock: liveClock() });
    const notes = await screen.findAllByTestId("home-tile-note");
    expect(notes).toHaveLength(4);
    for (const note of notes) expect(note).toHaveAttribute("title", note.textContent);
    // The total tile's note is its character count (docs/dictation.md §4.5).
    expect(notes[3]).toHaveTextContent(/^[\d,]+ 字$/);
  });

  it("regression: each stat tile takes its span's hue and the total is the lit ink card", async () => {
    // User decision 2026-10-10 (Penpot 「Voltip 统计卡」, option C, total ①): today, this week and
    // this month in the accent, ok and warning hues, the total an ink card lit by the accent; the
    // themes pick the colours through tokens.css. Board 01 (0.1.1) keeps the hues on 100 px tiles.
    renderApp({ mock: liveClock() });
    const values = await screen.findAllByTestId("home-tile-value");
    expect(values).toHaveLength(4);
    const tiles = values.map((value) => value.closest('[role="button"]'));
    expect(tiles.map((tile) => tile?.getAttribute("data-span"))).toEqual([
      "today",
      "week",
      "month",
      "all",
    ]);
    expect(tiles[0]).toHaveClass("stat-tile", "stat-tile-today", "min-h-[100px]");
    expect(tiles[1]).toHaveClass("stat-tile", "stat-tile-week");
    expect(tiles[2]).toHaveClass("stat-tile", "stat-tile-month");
    expect(tiles[3]).toHaveClass("stat-tile-total");
    expect(tiles[3]).not.toHaveClass("stat-tile");
    expect(tiles[0]).toHaveAccessibleName(/^今天 节省 /);
    for (const value of values) expect(value).toHaveClass("text-[26px]", "font-semibold");
    const icons = screen.getAllByTestId("home-tile-icon");
    expect(icons.map((icon) => icon.querySelector("svg")?.dataset.icon)).toEqual([
      "sun",
      "calendar",
      "calendarGrid",
      "clock",
    ]);
    for (const count of screen.getAllByTestId("home-tile-count"))
      expect(count).toHaveTextContent(/^听写 [\d,]+ 次$/);
    for (const caption of screen.getAllByTestId("home-tile-caption"))
      expect(caption).toHaveTextContent(/^节省时间[\d,]+ 字$/);
  });

  it("regression: a recent row's line under the text names the time, the app and each timing once", async () => {
    // 2026-10-11: the first draft of the rows read 「识别 412 ms ms」.
    const now = Date.now();
    renderApp({
      backend: new MockBackend({
        now: () => now,
        history: [
          {
            id: "with-app",
            at_ms: now - 60_000,
            raw_text: "把这段逻辑抽成一个 helper",
            text: "把这段逻辑抽成一个 helper。",
            refined: true,
            asr_model: "Qwen/Qwen3-ASR-1.7B",
            refine_model: "qwen/qwen3.8-27b",
            duration_ms: 6800,
            asr_ms: 412,
            refine_ms: 210,
            outcome: { kind: "inserted", via: "paste" },
            starred: false,
            mode: "whole_take",
            kind: "dictation",
            app: { id: "com.microsoft.VSCode", name: "VS Code" },
          },
        ],
      }),
    });
    const row = firstRow(await recentRows());
    const meta = within(row).getByTestId("home-row-meta");
    expect(meta).toHaveTextContent(/ · VS Code · 6\.8 s · 识别 412 ms · 润色 210 ms$/);
    expect(meta.textContent).not.toMatch(/ms ms/);
    // The app's letter tile, in a hue that stays with the app.
    const tile = within(row).getByTestId("home-app");
    expect(tile).toHaveTextContent("V");
    for (const cls of appHue("com.microsoft.VSCode").split(" ")) expect(tile).toHaveClass(cls);
    expect(appHue("com.microsoft.VSCode")).toBe(appHue("com.microsoft.VSCode"));
    expect(within(row).getByTestId("home-outcome")).toHaveTextContent("已插入 · 粘贴");
  });

  it("renders the 听写 card, the 识别与润色 card, the tiles and the recent results from the core's state", async () => {
    const { backend } = renderApp({ mock: liveClock() });
    expect(await screen.findByText("可以开始听写")).toBeInTheDocument();
    expect(screen.getByTestId("home-phase")).toHaveTextContent(/^麦克风、快捷键和识别服务已就绪$/);
    // Board 01: the chord in keycaps, how it drives a take, and where the text goes.
    expect(screen.getByTestId("home-hotkey")).toHaveAccessibleName("Ctrl Alt Space");
    expect(screen.getByTestId("home-activation")).toHaveTextContent("按住说话");
    expect(screen.getByTestId("home-hint")).toHaveTextContent(
      "松开后，文字插入到当前光标处 · 按 Esc 取消",
    );
    expect(screen.getByTestId("home-mic-state")).toHaveTextContent("空闲 · 未打开麦克风");
    // The engine card reads state.engines, not a fixture.
    const engine = screen.getByTestId("home-engine");
    expect(within(engine).getByText("识别与润色")).toBeInTheDocument();
    expect(within(engine).getByText("Qwen3-ASR-1.7B")).toBeInTheDocument();
    expect(within(engine).getByText(/内置服务 · 语言 自动/)).toBeInTheDocument();
    expect(within(engine).getByText("qwen3.8-27b")).toBeInTheDocument();
    expect(within(engine).getByRole("switch", { name: /AI 润色 开/ })).toBeChecked();
    expect(screen.getByTestId("home-privacy")).toHaveTextContent("文本发送到内置服务");
    await waitFor(() => {
      expect(within(engine).getByTestId("home-mic-device")).toHaveTextContent(
        MOCK_AUDIO_DEVICES[0]?.name ?? "",
      );
    });
    expect(within(engine).getByText(/48 kHz/)).toBeInTheDocument();
    // Regression (public release, 2026-09-27): the home page never names the built-in host.
    expect(screen.getByTestId("page-home").textContent).not.toMatch(/voltip\.example|groq\.com/);
    expect(
      [...document.querySelectorAll("[title]")].map((e) => e.getAttribute("title")).join(" "),
    ).not.toMatch(/voltip\.example/);
    // The panels the board left out are gone from the page.
    for (const gone of ["home-mic", "home-devices", "home-session", "home-mic-source"])
      expect(screen.queryByTestId(gone)).toBeNull();
    expect(screen.queryByRole("meter", { name: "强度" })).toBeNull();

    // Stats and tiles come from the core's history_stats (the sample rows dated relative to now):
    // each tile names the time saved, speaking time × 1.9 (docs/dictation.md §4.5).
    const stats = await homeStatsOf(backend);
    expect(stats.today.count).toBe(2);
    const saved = (ms: number) => `节省 ${formatDuration(ms)}`;
    expect(
      await screen.findByRole("button", { name: `今天 ${saved(stats.today.savedMs)}` }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `本周 ${saved(stats.week.savedMs)}` }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `本月 ${saved(stats.month.savedMs)}` }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `总计 ${saved(stats.total.savedMs)}` }),
    ).toBeInTheDocument();
    // The title bar's line under 首页 counts today and the whole history.
    expect(await screen.findByTestId("title-bar-summary")).toHaveTextContent(
      `今天听写 2 次 · 节省 ${formatDuration(stats.today.savedMs)} · 历史记录 ${formatCount(backend.peek().history_total)} 条`,
    );
    const card = screen.getByTestId("home-recent");
    const rows = await recentRows();
    expect(rows.length).toBeGreaterThanOrEqual(4);
    // Grouped by day, today first.
    expect(within(card).getAllByRole("heading", { level: 3 })[0]).toHaveTextContent(/^今天 · /);
    expect(within(card).getByText(/attach the latency report/)).toBeInTheDocument();
    // A clipboard fallback's chip is the short label; the reason is explained in the history entry.
    expect(within(card).getByText("已复制到剪贴板")).toBeInTheDocument();
    expect(within(card).queryByText(/目标窗口没有焦点/)).toBeNull();
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
      const halo = screen.getByTestId("home-halo");
      // Idle: nothing opens the microphone, the rings rest and the line says why.
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 5);
      });
      expect(backend.activeMeters()).toBe(0);
      expect(screen.getByTestId("home-mic-state")).toHaveTextContent("空闲 · 未打开麦克风");
      expect(halo.style.transform).toBe("scale(0.860)");
      // No 电平 anywhere.
      expect(screen.getByTestId("page-home").textContent).not.toMatch(/电平/);
      // 测试麦克风: the meter runs, the rings follow it, and it closes by itself after 15 s.
      await user.click(screen.getByTestId("home-mic-test"));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 2 + 1);
      });
      await waitFor(() => {
        expect(Number(/scale\(([\d.]+)\)/.exec(halo.style.transform)?.[1])).toBeGreaterThan(0.86);
      });
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

  // User request 2026-09-30: the device is switched in place. The device name is the menu; 录音来源
  // 设置… in it opens 设置 › 录音来源.
  it("regression: the device name opens the device menu, whose 录音来源设置… opens 设置 › 录音来源, and an unplugged choice is named as such", async () => {
    const user = userEvent.setup();
    renderApp({ mock: { settings: { microphone: "Blue Yeti" } } });
    expect(await screen.findByTestId("home-mic-missing")).toHaveTextContent(
      "所选麦克风未连接 · 使用系统默认",
    );
    const device = screen.getByTestId("home-mic-device");
    expect(device).toHaveTextContent(MOCK_AUDIO_DEVICES[0]?.name ?? "");
    await user.click(device);
    const menu = screen.getByTestId("home-mic-device-menu");
    // The chosen microphone that is not plugged in: named, checked, not choosable.
    const missing = within(menu).getByRole("menuitemradio", { name: "Blue Yeti · 未连接" });
    expect(missing).toHaveAttribute("aria-checked", "true");
    expect(missing).toBeDisabled();
    await user.click(within(menu).getByRole("menuitem", { name: "录音来源设置…" }));
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(
      within(dialog).getByRole("tab", { name: "录音来源", selected: true }),
    ).toBeInTheDocument();
    expect(within(dialog).getByTestId("microphone-pane")).toBeInTheDocument();
  });

  it("regression: 开始听写 starts a real session and shows the phase; 停止 finishes with the inserted text", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({ mock: liveClock() });
      await screen.findByText("可以开始听写");
      const before = backend.peek().history_recent.length;
      const start = screen.getByRole("button", { name: "开始听写" });
      expect(start).toBeEnabled();
      expect(start).not.toHaveAttribute("title");
      await user.click(start);
      expect(backend.peek().dictation.phase.phase).toBe("listening");
      expect(screen.getByText("正在听写")).toBeInTheDocument();
      expect(screen.getByTestId("home-phase")).toHaveTextContent(/正在录音… 00:0\d/);
      expect(screen.queryByRole("button", { name: "开始听写" })).toBeNull();
      // The line under the button names the recording.
      await waitFor(() => {
        expect(screen.getByTestId("home-mic-state")).toHaveTextContent("录音中");
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
      // The result landed in history: the list, the tiles and the sidebar count all moved.
      expect(backend.peek().history_recent).toHaveLength(before + 1);
      expect(firstRow(await recentRows())).toHaveTextContent(MOCK_DICTATION_TEXT);
      const after = await homeStatsOf(backend);
      expect([after.today.count, after.total.count]).toEqual([3, before + 1]);
      expect(
        await screen.findByRole("button", {
          name: `今天 节省 ${formatDuration(after.today.savedMs)}`,
        }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: `总计 节省 ${formatDuration(after.total.savedMs)}` }),
      ).toBeInTheDocument();
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

  it("regression: a voice edit reads as one on the home page while listening and rewriting and once replaced and its row is badged in the recent results (section 19)", async () => {
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
      const rows = await recentRows();
      const newest = firstRow(rows);
      expect(newest).toHaveTextContent(MOCK_EDIT_TEXT);
      expect(within(newest).getByTestId("home-edit-badge")).toHaveTextContent("编辑");
      expect(
        within(screen.getByTestId("home-recent")).getAllByTestId("home-edit-badge"),
      ).toHaveLength(1);
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
      const before = backend.peek().history_recent.length;
      await user.click(screen.getByRole("button", { name: "开始听写" }));
      await user.click(screen.getByRole("button", { name: "取消" }));
      expect(backend.peek().dictation.phase).toEqual({ phase: "cancelled", injected_chars: 0 });
      expect(screen.getByTestId("home-phase")).toHaveTextContent("已取消");
      expect(backend.peek().history_recent).toHaveLength(before);
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
  });

  it("regression: changing the hotkey updates the 听写 card's keycaps immediately", async () => {
    const { backend } = renderApp();
    const keys = await screen.findByTestId("home-hotkey");
    expect(keys).toHaveAccessibleName("Ctrl Alt Space");
    expect([...keys.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual([
      "Ctrl",
      "Alt",
      "Space",
    ]);
    await act(async () => {
      await backend.invoke("settings_set_hotkey", { hotkey: "Ctrl+Shift+D" });
    });
    expect(screen.getByTestId("home-hotkey")).toHaveAccessibleName("Ctrl Shift D");
    expect(
      [...screen.getByTestId("home-hotkey").querySelectorAll("kbd")].map((k) => k.textContent),
    ).toEqual(["Ctrl", "Shift", "D"]);
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
    expect(screen.getByTestId("home-phase")).toHaveClass("text-danger");
    expect(screen.getByRole("button", { name: "今天 节省 0 秒" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "总计 节省 0 秒" })).toBeInTheDocument();
    expect(await recentRows()).toHaveLength(0);
    expect(screen.getByText("暂无听写结果")).toBeInTheDocument();
    expect(screen.getByText("按住 Ctrl Alt Space 说一句，松开即插入")).toBeInTheDocument();
    expect(await screen.findByText("Fifine K669 USB Microphone")).toBeInTheDocument();
  });

  it("regression: the activation chip, the hint and the empty-state hint follow settings.activation (docs/dictation.md §13)", async () => {
    const { backend } = renderApp({
      mock: { history: [], settings: { activation: "toggle" } },
    });
    expect(
      await screen.findByRole("button", { name: "按一下开始 · 再按结束" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("home-hint")).toHaveTextContent(
      "再按一次结束，文字插入到当前光标处 · 按 Esc 取消",
    );
    expect(screen.getByText("按一下 Ctrl Alt Space 开始，再按一下结束")).toBeInTheDocument();
    expect(screen.queryByText("按住说话")).toBeNull();
    // The core switches the mode (settings event): chip and hints follow at once.
    act(() => {
      backend.publish({
        type: "settings",
        ...backend.peek().settings,
        activation: "hold_or_toggle",
      });
    });
    expect(await screen.findByRole("button", { name: "按住说话 · 短按锁定" })).toBeInTheDocument();
    expect(screen.getByTestId("home-hint")).toHaveTextContent(
      "松开即结束，短按锁定继续录音 · 文字插入到当前光标处",
    );
    expect(screen.getByText("按住 Ctrl Alt Space 说话，短按锁定")).toBeInTheDocument();
    act(() => {
      backend.publish({ type: "settings", ...backend.peek().settings, activation: "hold" });
    });
    expect(await screen.findByRole("button", { name: "按住说话" })).toBeInTheDocument();
    expect(screen.getByText("按住 Ctrl Alt Space 说一句，松开即插入")).toBeInTheDocument();
  });

  it("the hint says the text is only copied when the output mode is the clipboard", async () => {
    renderApp({
      mock: {
        settings: { engines: { ...defaultEngineSettings(), inject: "clipboard_only" } },
      },
    });
    await waitFor(() => {
      expect(screen.getByTestId("home-hint")).toHaveTextContent(
        "松开后，文字复制到剪贴板 · 按 Esc 取消",
      );
    });
  });

  it("regression: the Bridge & MCP card is gone and nothing on the page names it", async () => {
    renderApp();
    const page = await screen.findByTestId("page-home");
    // Windows test 2026-09-25: Bridge & MCP is removed, nothing on the page may mention it.
    expect(page.textContent).not.toMatch(/Bridge|MCP|Claude Code|OpenCode|Hook/);
    expect(
      screen.queryByRole("button", { name: /复制 MCP 配置|复制 Hook 命令|待批准/ }),
    ).toBeNull();
  });

  it("regression: home is fluid (no fixed-width panels)", async () => {
    renderApp();
    const page = await screen.findByTestId("page-home");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1600px]", "p-6");
    expect(fixedSizeOffenders(page)).toEqual([]);
    for (const grid of page.querySelectorAll<HTMLElement>(".grid"))
      expect(grid.className).not.toMatch(/(?:^|\s)(?:lg:|xl:)?grid-cols-\[[^\]]*\d+px/);
  });

  it("the AI 润色 toggle writes settings_set_engines and the card follows the core", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp();
    await screen.findByText("可以开始听写");
    const card = screen.getByTestId("home-engine");
    await user.click(within(card).getByRole("switch", { name: /AI 润色 开/ }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_enabled).toBe(false);
    });
    expect(backend.peek().engines.refine_enabled).toBe(false);
    expect(within(card).getByRole("switch", { name: "AI 润色 关" })).not.toBeChecked();
    // Off, no text leaves for clean-up, and the card no longer says it does.
    expect(screen.queryByTestId("home-privacy")).toBeNull();
    // The 润色模型 menu keeps naming the model (it is the menu that switches it).
    expect(within(card).getByTestId("home-refine-model")).toHaveTextContent("qwen3.8-27b");
    // 配置 opens the 语音模型 page (a page of the main layout since 2026-09-28).
    await user.click(within(card).getByTestId("home-configure"));
    expect(await screen.findByTestId("page-speech")).toBeInTheDocument();
    expect(screen.getByTestId("speech-pane")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    await user.click(screen.getByRole("button", { name: /^首页$/ }));
    await user.click(screen.getByRole("button", { name: "查看全部 →" }));
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
  });

  it("the engine card names the AI preset and its menu switches it (docs/dictation.md section 21)", async () => {
    const user = userEvent.setup();
    const weekly = {
      id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
      name: "周报",
      prompt: "整理成周报",
      created_at_ms: 1,
      updated_at_ms: 1,
    };
    const { backend } = renderApp({ mock: { presets: [weekly] } });
    await screen.findByText("可以开始听写");
    const card = screen.getByTestId("home-engine");
    expect(within(card).getByTestId("home-engine-preset")).toHaveTextContent("校对");
    await user.click(within(card).getByRole("button", { name: "AI 预设：校对" }));
    const menu = within(card).getByRole("menu", { name: "AI 预设" });
    expect(
      within(menu)
        .getAllByRole("menuitemradio")
        .map((i) => i.textContent),
    ).toEqual([
      "校对",
      "提示词优化",
      "意图整理",
      "口语聊天",
      "中英互译",
      "要点纪要",
      "只加标点",
      "书面语",
      "周报",
    ]);
    await user.click(within(menu).getByRole("menuitemradio", { name: "周报" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_preset).toBe(weekly.id);
    });
    expect(within(card).getByRole("button", { name: "AI 预设：周报" })).toBeInTheDocument();
    expect(within(card).getByTestId("home-engine-preset")).toHaveTextContent("周报");
    expect(within(card).getByTestId("home-engine-preset")).toHaveAttribute("data-user-text");
    // With AI polish off the menu stays and says so on hover.
    await user.click(within(card).getByRole("switch", { name: /AI 润色 开/ }));
    await waitFor(() => {
      expect(backend.peek().engines.refine_enabled).toBe(false);
    });
    expect(within(card).getByRole("button", { name: "AI 预设：周报" })).toHaveAttribute(
      "title",
      "AI 润色 关",
    );
    // 管理预设… opens the presets of the AI 模型 page.
    await user.click(within(card).getByRole("button", { name: "AI 预设：周报" }));
    await user.click(within(card).getByRole("menuitem", { name: "管理预设…" }));
    expect(await screen.findByTestId("presets-section")).toBeInTheDocument();
  });

  it("stat tiles, the activation chip, the model menu and recent rows navigate on every platform", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      backend: new MockBackend({
        identity: { ...phoneIdentity(), platform: "macos" },
        now: () => Date.now(),
      }),
    });
    await screen.findByText("可以开始听写");
    expect(screen.getByRole("button", { name: "开始听写" })).toBeEnabled();
    const stats = await homeStatsOf(backend);
    const saved = (ms: number) => `节省 ${formatDuration(ms)}`;
    await user.click(
      await screen.findByRole("button", { name: `本周 ${saved(stats.week.savedMs)}` }),
    );
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /首页/ }));
    // Settings open as a modal over the page (Shell); 按住说话 lands on the 快捷键 group.
    await user.click(screen.getByRole("button", { name: "按住说话" }));
    const settings = screen.getByRole("dialog", { name: "设置" });
    expect(within(settings).getByRole("tab", { name: /快捷键/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
    // The model is a menu (user request 2026-09-30): its 管理语音模型… leads to the page.
    const model = screen.getByTestId("home-engine-model");
    expect(model).toHaveAccessibleName("语音模型：Qwen3-ASR-1.7B");
    await user.click(model);
    await user.click(
      within(screen.getByTestId("home-engine-model-menu")).getByRole("menuitem", {
        name: "管理语音模型…",
      }),
    );
    expect(await screen.findByTestId("page-speech")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^首页$/ }));
    const tile = await screen.findByRole("button", { name: `本月 ${saved(stats.month.savedMs)}` });
    tile.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /首页/ }));
    const latency = (await recentRows()).find((r) => /latency report/.test(r.textContent ?? ""));
    if (latency === undefined) throw new Error("fixture");
    await user.click(within(latency).getByTestId("home-row-open"));
    expect(screen.getByRole("heading", { name: "历史记录", level: 1 })).toBeInTheDocument();
    // The recent row's id pre-selects it in the history detail.
    expect(screen.getByTestId("entry-text")).toHaveTextContent(/attach the latency report/);
  });
});

describe("Home page · time saved (docs/dictation.md section 4.5)", () => {
  it("explains the 1.9 factor and its source in the 依据 popover beside the title bar's summary", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByText("可以开始听写");
    const basis = screen.getByRole("button", { name: "节省时间的依据" });
    expect(screen.getByTestId("title-bar-subtitle")).toContainElement(basis);
    await user.click(basis);
    const panel = screen.getByRole("dialog", { name: "节省时间的依据" });
    expect(panel).toHaveTextContent(
      "节省时间 = 说话时长 × 1.9。依据：Ruan 等 2016（arXiv:1608.07323）的实验里，说话输入比手机打字快约 2.9 倍（英文 153 vs 52 词/分，中文 123 vs 43）。只统计保存在历史记录里的听写。",
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "节省时间的依据" })).toBeNull();
    expect(basis).toHaveFocus();
  });
});
