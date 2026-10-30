// Importing audio and video files on the history page (docs/dictation.md §31), on the whole app
// with the in-memory core: 导入音频或视频 and its options (never the built-in service), a drop that
// takes only the media files and only on this page, the queue's states, and an imported entry's
// marks and exports.
import { type HistoryEntry, type ImportJob, defaultEngineSettings } from "@voltip/shared";
import {
  MOCK_IMPORT_FILE,
  MOCK_IMPORT_PICK,
  MOCK_IMPORT_SEGMENT_MS,
  MOCK_IMPORT_STEP_MS,
  MOCK_EXPORT_DIR,
  MockBackend,
  type MockBackendOptions,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

/** Fake timers that still advance with the wall clock; `user` advances them while it clicks. */
function withTimers() {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  return userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
}

afterEach(() => {
  vi.useRealTimers();
});

/** `MOCK_IMPORT_FILE`'s segments, and the steps of a whole job without polish. */
const SEGMENTS = Math.ceil(MOCK_IMPORT_FILE.duration_ms / MOCK_IMPORT_SEGMENT_MS);
const WHOLE_MS = (6 + SEGMENTS) * MOCK_IMPORT_STEP_MS;

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** A desktop core with the user's own Groq key for both services, unless told otherwise. */
function core(options: MockBackendOptions = {}) {
  return new MockBackend({
    now: () => Date.now(),
    history: [],
    providerKeys: [
      { provider: "groq", kind: "asr" },
      { provider: "groq", kind: "llm" },
    ],
    ...options,
  });
}

function imported(extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: "00000000-0000-4000-8000-000000000031",
    at_ms: Date.now() - 60_000,
    raw_text: "嗯今天过一下发布安排然后周四灰度",
    text: "今天过一下发布安排。周四灰度。",
    refined: true,
    asr_model: "whisper-large-v3",
    refine_model: "llama-3.3-70b-versatile",
    duration_ms: 24_000,
    asr_ms: 900,
    refine_ms: 700,
    outcome: { kind: "inserted", via: "clipboard" },
    starred: false,
    mode: "whole_take",
    segments: [
      {
        text: "嗯今天过一下发布安排",
        refined: "今天过一下发布安排。",
        start_ms: 0,
        end_ms: 12_000,
      },
      { text: "然后周四灰度", refined: "周四灰度。", start_ms: 12_000, end_ms: 24_000 },
    ],
    kind: "dictation",
    file: { name: "team-sync.mp4", duration_ms: 24_000 },
    ...extra,
  };
}

/** The values of a select's options. */
function optionValues(select: HTMLElement): string[] {
  return [...select.querySelectorAll("option")].map((o) => o.value);
}

describe("History · 导入音频或视频", () => {
  it("picks files, offers only the user's own models, and the files go through the queue into the history", async () => {
    const user = withTimers();
    const backend = core();
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    expect(backend.importPicks).toEqual([
      { title: "导入音频或视频", media: "音频和视频文件", all: "所有文件" },
    ]);
    const dialog = await screen.findByTestId("import-dialog");
    expect(within(dialog).getByTestId("import-files")).toHaveTextContent(
      ["team-sync.mp4", "interview.m4a"].join(""),
    );
    // The settings use the built-in service: the list starts on the first model of the user's own.
    const speech = within(dialog).getByTestId("import-speech");
    const values = optionValues(speech);
    expect(values.length).toBeGreaterThan(0);
    expect(values.some((v) => v.startsWith("remote:builtin:"))).toBe(false);
    expect(values.every((v) => v.startsWith("remote:groq:"))).toBe(true);
    expect((speech as HTMLSelectElement).value).toBe(values[0]);
    // The polish is off until it is turned on.
    expect(within(dialog).queryByTestId("import-polish-model")).toBeNull();
    await user.click(screen.getByTestId("import-start"));
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    const queue = await screen.findByTestId("import-queue");
    await waitFor(() => {
      expect(within(queue).getAllByTestId("import-job")).toHaveLength(2);
    });
    const jobs = () => within(queue).getAllByTestId("import-job");
    expect(jobs().map((j) => j.dataset.state)).toEqual(["decoding", "queued"]);
    expect(within(queue).getByTestId("import-waiting")).toHaveTextContent("另有 1 个文件等待导入");
    expect(backend.peek().settings.engines.asr_provider).toBe("builtin");
    await advance(6 * MOCK_IMPORT_STEP_MS);
    expect(jobs()[0]).toHaveAttribute("data-state", "recognizing");
    expect(within(jobs()[0] ?? queue).getByTestId("import-job-state")).toHaveTextContent(
      `正在识别，已完成 1/${SEGMENTS} 段`,
    );
    expect(within(jobs()[0] ?? queue).getByRole("progressbar")).toBeInTheDocument();
    await advance(2 * WHOLE_MS);
    expect(jobs().map((j) => j.dataset.state)).toEqual(["done", "done"]);
    expect(within(jobs()[0] ?? queue).getByTestId("import-job-state")).toHaveTextContent(
      "已完成，已保存到历史记录",
    );
    // Each file is an entry of its own, marked and named in the log.
    const log = screen.getByRole("list", { name: "听写记录" });
    await waitFor(() => {
      expect(
        within(log)
          .getAllByTestId("history-file")
          .map((f) => f.textContent),
      ).toEqual(["导入interview.m4a", "导入team-sync.mp4"]);
    });
    expect(within(log).getAllByTestId("history-row-meta")[0]).toHaveTextContent("从文件导入");
    // 查看记录 opens the first file's entry; 移除 takes the job out of the queue only.
    await user.click(within(jobs()[0] ?? queue).getByTestId("import-view"));
    await waitFor(() => {
      expect(screen.getByTestId("history-detail-file-name")).toHaveTextContent("team-sync.mp4");
    });
    expect(screen.getByTestId("history-detail-file")).toHaveTextContent("导入");
    expect(screen.getByTestId("history-long-tools")).toBeInTheDocument();
    await user.click(within(jobs()[0] ?? queue).getByTestId("import-remove"));
    await user.click(within(jobs()[0] ?? queue).getByTestId("import-remove"));
    await waitFor(() => {
      expect(screen.queryByTestId("import-queue")).toBeNull();
    });
    expect(backend.peek().history_total).toBe(2);
  });

  it("turns the polish on with the user's own AI model only, and with none leads to AI 模型", async () => {
    const user = withTimers();
    const backend = core();
    backend.simulateImportPick(["/home/you/Videos/talk.webm"]);
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    const dialog = await screen.findByTestId("import-dialog");
    await user.click(within(dialog).getByRole("switch", { name: "AI 润色" }));
    const polish = within(dialog).getByTestId("import-polish-model");
    const values = optionValues(polish);
    expect(values.length).toBeGreaterThan(0);
    expect(values.some((v) => v.startsWith("remote:builtin:"))).toBe(false);
    await user.selectOptions(polish, values.at(-1) ?? "");
    await user.click(screen.getByTestId("import-start"));
    await advance(10 * WHOLE_MS);
    const [entry] = backend.peek().history_recent;
    expect(entry?.file?.name).toBe("talk.webm");
    expect(entry?.refined).toBe(true);
    expect(entry?.refine_model).toBe(values.at(-1)?.replace("remote:groq:", ""));
    expect(entry?.segments?.every((s) => s.refined !== undefined)).toBe(true);
  });

  it("without an AI key of the user's own there is no polish to turn on: the dialog leads to AI 模型", async () => {
    const user = withTimers();
    // A vendor's two services share one key: the recognition here is an installed local model.
    const id = "sense-voice-small";
    const backend = core({
      providerKeys: [],
      models: { [id]: { kind: "installed", path: `/models/${id}`, installed_at: 1 } },
    });
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    const dialog = await screen.findByTestId("import-dialog");
    await user.click(within(dialog).getByRole("switch", { name: "AI 润色" }));
    expect(within(dialog).getByTestId("import-no-polish")).toHaveTextContent(
      "没有可以用于导入的润色模型",
    );
    expect(screen.getByTestId("import-start")).toBeDisabled();
    // Turned off again, the import can start without a polish.
    await user.click(within(dialog).getByRole("switch", { name: "AI 润色" }));
    expect(screen.getByTestId("import-start")).toBeEnabled();
    await user.click(within(dialog).getByRole("switch", { name: "AI 润色" }));
    await user.click(within(dialog).getByRole("button", { name: "管理 AI 模型" }));
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    expect(await screen.findByRole("heading", { name: "AI 模型", level: 1 })).toBeInTheDocument();
  });

  it("with no model of the user's own, says so, leads to 语音模型 and does not start", async () => {
    const user = withTimers();
    const backend = core({ providerKeys: [] });
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    const dialog = await screen.findByTestId("import-dialog");
    expect(within(dialog).getByTestId("import-no-speech")).toHaveTextContent(
      "没有可以用于导入的语音模型",
    );
    expect(screen.getByTestId("import-start")).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "管理语音模型" }));
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    expect(await screen.findByRole("heading", { name: "语音模型", level: 1 })).toBeInTheDocument();
    expect(backend.peek().imports.jobs).toEqual([]);
  });

  it("an installed local model is offered and the language goes with the files", async () => {
    const user = withTimers();
    const id = "sense-voice-small";
    const backend = core({
      providerKeys: [],
      models: { [id]: { kind: "installed", path: `/models/${id}`, installed_at: 1 } },
    });
    const sent: unknown[] = [];
    const invoke = backend.invoke.bind(backend);
    vi.spyOn(backend, "invoke").mockImplementation(async (name, ...args) => {
      if (name === "import_add") sent.push(args[0]);
      await invoke(name, ...args);
    });
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    const dialog = await screen.findByTestId("import-dialog");
    expect(optionValues(within(dialog).getByTestId("import-speech"))).toEqual([`local:${id}`]);
    await user.selectOptions(within(dialog).getByTestId("import-language"), "en");
    await user.click(screen.getByTestId("import-start"));
    expect(sent).toEqual([
      { paths: [...MOCK_IMPORT_PICK], asr: { kind: "local", id }, language: "en" },
    ]);
  });

  it("a drop on the history page takes the audio and video files and names the others; other pages ignore it", async () => {
    const user = withTimers();
    const backend = core();
    renderApp({ path: "/history", backend });
    await screen.findByTestId("history-import");
    // The page listens once it has rendered.
    await waitFor(() => {
      expect(backend.fileDropListeners()).toBe(1);
    });
    act(() => {
      backend.simulateFileDrop(["/v/a.MP4", "/v/notes.txt"]);
    });
    // regression (2026-10-30, the real-entry check): the skipped file was named only in a toast,
    // which the options dialog opening at the same moment covered. The dialog names it.
    const dialog = await screen.findByTestId("import-dialog");
    expect(within(dialog).getByTestId("import-files")).toHaveTextContent("a.MP4");
    expect(within(dialog).getByTestId("import-skipped")).toHaveTextContent(
      "已跳过 notes.txt：不是音频或视频文件",
    );
    // A second drop while the options are open adds its files, and counts what it skipped.
    act(() => {
      backend.simulateFileDrop(["C:\\Videos\\b.mkv", "/v/c.pdf", "/v/d"]);
    });
    await waitFor(() => {
      expect(within(dialog).getByTestId("import-files")).toHaveTextContent("a.MP4b.mkv");
    });
    expect(within(dialog).getByTestId("import-skipped")).toHaveTextContent(
      "已跳过 3 个不是音频或视频的文件",
    );
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    expect(backend.peek().imports.jobs).toEqual([]);
    // Without a file to import there is no dialog: the notice is a toast, short enough to fit it.
    act(() => {
      backend.simulateFileDrop(["/v/notes.txt"]);
    });
    expect(await screen.findByText("已跳过 notes.txt：不是音频或视频文件")).toBeInTheDocument();
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    // On another page a drop does nothing.
    await user.click(screen.getByRole("button", { name: /^首页/ }));
    await screen.findByTestId("page-home");
    expect(backend.fileDropListeners()).toBe(0);
    act(() => {
      backend.simulateFileDrop(["/v/e.mp4"]);
    });
    expect(screen.queryByTestId("import-dialog")).toBeNull();
  });

  it("with the history off, neither the button nor a drop opens the options", async () => {
    const user = withTimers();
    const backend = core({ settings: { history: { enabled: false, keep: 200 } } });
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    // regression (2026-10-30): the notice was longer than the toast and lost its end; it is short
    // now, and its button opens the setting.
    expect(await screen.findByText("导入需要先开启「保存听写历史」")).toBeInTheDocument();
    await waitFor(() => {
      expect(backend.fileDropListeners()).toBe(1);
    });
    act(() => {
      backend.simulateFileDrop(["/v/a.mp4"]);
    });
    expect(screen.queryByTestId("import-dialog")).toBeNull();
    expect(backend.importPicks).toEqual([]);
    await user.click(screen.getAllByRole("button", { name: "去设置" })[0] as HTMLElement);
    expect(
      await screen.findByRole("heading", { name: "隐私与历史", level: 2 }),
    ).toBeInTheDocument();
  });

  it("the queue shows each state: waiting, running, paused for a dictation, failed with 重试 and 移除, done", async () => {
    const user = withTimers();
    const job = (
      id: number,
      state: ImportJob["state"],
      extra: Partial<ImportJob> = {},
    ): ImportJob => ({
      id: `0b0c3a52-6c1e-4b3e-9a51-${String(id).padStart(12, "0")}`,
      name: `file-${id}.mp4`,
      size: 18_400_000,
      paused: false,
      state,
      ...extra,
    });
    const backend = core({
      imports: [
        job(1, { kind: "recognizing", done: 3, total: 9 }, { paused: true, duration_ms: 100_000 }),
        job(2, { kind: "queued" }),
        job(3, {
          kind: "failed",
          failure: { code: "unsupported_codec", codec: "AC-3" },
          resumable: false,
        }),
        job(4, {
          kind: "failed",
          failure: { code: "recognition", detail: "timed out" },
          resumable: true,
        }),
        job(5, { kind: "done", entry: "00000000-0000-4000-8000-000000000099", polish_failed: 2 }),
      ],
    });
    renderApp({ path: "/history", backend });
    const queue = await screen.findByTestId("import-queue");
    const rows = within(queue).getAllByTestId("import-job");
    const texts = rows.map((r) => within(r).getByTestId("import-job-state").textContent);
    expect(texts).toEqual([
      "正在识别，已完成 3/9 段 · 正在听写，导入将在听写结束后继续",
      "等待中",
      "不支持这种音频编码：AC-3",
      "识别失败：timed out",
      "已完成，2 段润色失败，保留了识别原文",
    ]);
    expect(rows[0]).toHaveTextContent("18.4 MB · 1 分 40 秒");
    expect(within(rows[0] ?? queue).getByTestId("import-cancel")).toBeInTheDocument();
    expect(within(rows[1] ?? queue).getByTestId("import-cancel")).toBeInTheDocument();
    expect(within(rows[2] ?? queue).getByTestId("import-retry")).not.toHaveAttribute("title");
    expect(within(rows[3] ?? queue).getByTestId("import-retry")).toHaveAttribute(
      "title",
      "重试会从未完成的段继续",
    );
    expect(within(rows[4] ?? queue).getByTestId("import-view")).toBeInTheDocument();
    expect(queue).toHaveTextContent("关闭 Voltip 会清空导入列表，原文件不受影响。");
    await user.click(within(rows[1] ?? queue).getByTestId("import-cancel"));
    await user.click(within(rows[2] ?? queue).getByTestId("import-remove"));
    await waitFor(() => {
      expect(within(queue).getAllByTestId("import-job")).toHaveLength(3);
    });
  });

  it("a running import waits while a dictation runs and says so", async () => {
    const user = withTimers();
    const backend = core();
    backend.simulateImportPick(["/v/long.mkv"]);
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-import"));
    await user.click(await screen.findByTestId("import-start"));
    await advance(6 * MOCK_IMPORT_STEP_MS);
    await act(async () => {
      await backend.invoke("dictation_start");
    });
    await advance(MOCK_IMPORT_STEP_MS);
    const row = within(screen.getByTestId("import-queue")).getByTestId("import-job");
    expect(row).toHaveAttribute("data-paused", "true");
    expect(within(row).getByTestId("import-paused")).toHaveTextContent(
      "正在听写，导入将在听写结束后继续",
    );
    await act(async () => {
      await backend.invoke("dictation_cancel");
    });
    await advance(2 * MOCK_IMPORT_STEP_MS);
    expect(row).toHaveAttribute("data-paused", "false");
  });
});

describe("History · an imported entry", () => {
  it("is marked in the log and the detail, and its exports take the file's name and the subtitle text chosen", async () => {
    const user = withTimers();
    const entry = imported();
    const backend = core({ history: [entry] });
    renderApp({ path: "/history", backend });
    const log = await screen.findByRole("list", { name: "听写记录" });
    expect(await within(log).findByTestId("history-file")).toHaveTextContent("导入team-sync.mp4");
    expect(within(log).getByTestId("history-row-meta")).toHaveTextContent("24.0 s · 从文件导入");
    expect(screen.getByTestId("history-detail-file-name")).toHaveTextContent("team-sync.mp4");
    const subtitles = screen.getByTestId("history-export-subtitles");
    expect((subtitles as HTMLSelectElement).value).toBe("refined");
    await user.click(screen.getByTestId("history-export-srt"));
    await user.selectOptions(subtitles, "raw");
    await user.click(screen.getByTestId("history-export-srt"));
    await user.click(screen.getByTestId("history-export-txt"));
    expect(backend.exports).toEqual([
      { id: entry.id, format: "srt", fileName: "team-sync", subtitles: "refined" },
      { id: entry.id, format: "srt", fileName: "team-sync", subtitles: "raw" },
      { id: entry.id, format: "txt", fileName: "team-sync" },
    ]);
    expect(await screen.findByText(`已保存：${MOCK_EXPORT_DIR}/team-sync.txt`)).toBeInTheDocument();
  });

  it("regression: its timing is the file's import, not a shortcut take", async () => {
    // 2026-10-30, the real-entry check: the detail of an imported file said 耗时 · 从松开快捷键到插入.
    const backend = core({ history: [imported()] });
    renderApp({ path: "/history", backend });
    expect(await screen.findByText("耗时 · 导入这个文件")).toBeInTheDocument();
    expect(screen.queryByText("耗时 · 从松开快捷键到插入")).toBeNull();
  });

  it("without polished segments offers no subtitle choice and sends none", async () => {
    const user = withTimers();
    const entry = imported({
      refined: false,
      refine_model: undefined,
      refine_ms: undefined,
      text: "嗯今天过一下发布安排然后周四灰度",
      segments: [{ text: "嗯今天过一下发布安排然后周四灰度", start_ms: 0, end_ms: 24_000 }],
    });
    const backend = core({ history: [entry], settings: { engines: defaultEngineSettings() } });
    renderApp({ path: "/history", backend });
    await user.click(await screen.findByTestId("history-export-srt"));
    expect(screen.queryByTestId("history-export-subtitles")).toBeNull();
    expect(backend.exports).toEqual([{ id: entry.id, format: "srt", fileName: "team-sync" }]);
  });
});
