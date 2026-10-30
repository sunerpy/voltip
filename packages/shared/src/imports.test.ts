import { describe, expect, it } from "vitest";
import { createTranslator } from "./i18n";
import {
  baseName,
  extensionOf,
  hasPolishedSegments,
  importFailureText,
  importProgress,
  importStateText,
  isImportRunning,
  isImported,
  runningImport,
  skippedFilesText,
  splitDrop,
  stem,
} from "./imports";
import type { HistoryEntry, ImportFailure, ImportJob, ImportState } from "./schema";

const zh = createTranslator("zh-CN").t;
const en = createTranslator("en").t;

const job = (state: ImportState, name = "a.mp4"): ImportJob => ({
  id: `id-${name}`,
  name,
  size: 1,
  paused: false,
  state,
});

const entry = (extra: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id: "e1",
  at_ms: 0,
  raw_text: "原文",
  text: "原文",
  refined: false,
  asr_model: "m",
  duration_ms: 1000,
  asr_ms: 1,
  outcome: { kind: "inserted", via: "clipboard" },
  starred: false,
  mode: "whole_take",
  kind: "dictation",
  ...extra,
});

describe("import helpers (docs/dictation.md §31)", () => {
  it("names a file by the last part of its path, on either system", () => {
    expect(baseName("/home/you/Videos/team sync.mp4")).toBe("team sync.mp4");
    expect(baseName("C:\\Users\\you\\Music\\访谈.M4A")).toBe("访谈.M4A");
    expect(baseName("plain")).toBe("plain");
    expect(stem("team sync.mp4")).toBe("team sync");
    expect(stem("archive.tar.gz")).toBe("archive.tar");
    expect(stem(".hidden")).toBe(".hidden");
    expect(stem("noext")).toBe("noext");
    expect(extensionOf("C:\\Users\\you\\访谈.M4A")).toBe("m4a");
    expect(extensionOf("/v/no-extension")).toBe("");
    expect(extensionOf("/v/.webm")).toBe("");
  });

  it("splits a drop into the files the import takes and the others, in order", () => {
    const paths = ["/v/a.MP4", "/v/notes.txt", "C:\\b.mkv", "/v/c", "/v/d.wav"];
    expect(splitDrop(paths, ["mp4", "mkv", "WAV"])).toEqual({
      accepted: ["/v/a.MP4", "C:\\b.mkv", "/v/d.wav"],
      rejected: ["/v/notes.txt", "/v/c"],
    });
    expect(splitDrop([], ["mp4"])).toEqual({ accepted: [], rejected: [] });
    expect(splitDrop(["/v/a.mp4"], [])).toEqual({ accepted: [], rejected: ["/v/a.mp4"] });
  });

  it("finds the job that runs and how far it is", () => {
    const states: ImportState[] = [
      { kind: "queued" },
      { kind: "decoding" },
      { kind: "decoding", pct: 50 },
      { kind: "recognizing", done: 2, total: 4 },
      { kind: "recognizing", done: 0, total: 0 },
      { kind: "polishing", done: 1, total: 4 },
      { kind: "polishing", done: 0, total: 0 },
      { kind: "done", entry: "e1", polish_failed: 0 },
      { kind: "failed", failure: { code: "no_speech" }, resumable: false },
    ];
    expect(states.map(isImportRunning)).toEqual([
      false,
      true,
      true,
      true,
      true,
      true,
      true,
      false,
      false,
    ]);
    expect(states.map(importProgress)).toEqual([
      undefined,
      undefined,
      0.05,
      0.1 + 0.45,
      0.1,
      0.25,
      1,
      undefined,
      undefined,
    ]);
    const view = {
      available: true,
      extensions: [],
      jobs: [
        job({ kind: "done", entry: "e", polish_failed: 0 }, "x"),
        job(states[3] ?? { kind: "queued" }, "y"),
      ],
    };
    expect(runningImport(view)?.name).toBe("y");
    expect(runningImport({ ...view, jobs: [] })).toBeUndefined();
  });

  it("words every failure and every state in both languages", () => {
    const failures: ImportFailure[] = [
      { code: "unreadable", detail: "permission denied" },
      { code: "unsupported_format" },
      { code: "no_audio" },
      { code: "unsupported_codec", codec: "AC-3" },
      { code: "corrupt", detail: "cut off" },
      { code: "storage", detail: "disk full" },
      { code: "recognition", detail: "timed out" },
      { code: "no_speech" },
    ];
    expect(failures.map((f) => importFailureText(f, zh, "zh-CN"))).toEqual([
      "无法读取文件：permission denied",
      "不是可导入的音频或视频文件",
      "文件中没有音轨",
      "不支持这种音频编码：AC-3",
      "文件已损坏或不完整：cut off",
      "无法保存：disk full",
      "识别失败：timed out",
      "文件中没有识别到语音",
    ]);
    for (const f of failures) expect(importFailureText(f, en, "en")).not.toMatch(/[{}]|history\./);
    expect(importStateText({ kind: "queued" }, zh, "zh-CN")).toBe("等待中");
    expect(importStateText({ kind: "decoding" }, zh, "zh-CN")).toBe("正在读取音频");
    expect(importStateText({ kind: "decoding", pct: 40 }, en, "en")).toBe("Reading the audio 40%");
    expect(importStateText({ kind: "recognizing", done: 0, total: 3 }, zh, "zh-CN")).toBe(
      "正在识别，已完成 0/3 段",
    );
    expect(importStateText({ kind: "polishing", done: 2, total: 3 }, en, "en")).toBe(
      "Polishing, 2/3 segments done",
    );
    expect(importStateText({ kind: "done", entry: "e", polish_failed: 0 }, zh, "zh-CN")).toBe(
      "已完成，已保存到历史记录",
    );
    expect(importStateText({ kind: "done", entry: "e", polish_failed: 1 }, en, "en")).toBe(
      "Done; 1 segment could not be polished and kept the recognised text",
    );
    expect(importStateText({ kind: "done", entry: "e", polish_failed: 2 }, en, "en")).toContain(
      "2 segments",
    );
    expect(
      importStateText(
        { kind: "failed", failure: { code: "no_audio" }, resumable: false },
        en,
        "en",
      ),
    ).toBe("The file has no audio track");
  });

  it("names one skipped file and counts several, short enough for a toast", () => {
    expect(skippedFilesText(["notes.txt"], zh)).toBe("已跳过 notes.txt：不是音频或视频文件");
    expect(skippedFilesText(["a.pdf", "b"], zh)).toBe("已跳过 2 个不是音频或视频的文件");
    expect(skippedFilesText(["notes.txt"], en)).toBe("Skipped notes.txt: not audio or video");
    expect(skippedFilesText(["a.pdf", "b"], en)).toBe("Skipped 2 files: not audio or video");
  });

  it("regression: a recognition failure reads in the interface's language, not the client's", () => {
    // 2026-10-30, the real-entry check: the queue said 识别失败：asr: ASR request timed out.
    const timeout: ImportFailure = { code: "recognition", detail: "asr: ASR request timed out" };
    expect(importFailureText(timeout, zh, "zh-CN")).toBe("识别失败：请求超时");
    expect(importFailureText(timeout, en, "en")).toBe("Recognition failed: the request timed out");
    expect(
      importStateText({ kind: "failed", failure: timeout, resumable: true }, zh, "zh-CN"),
    ).toBe("识别失败：请求超时");
    const server: ImportFailure = {
      code: "recognition",
      detail: "asr: ASR server error 503: overloaded",
    };
    expect(importFailureText(server, zh, "zh-CN")).toBe(
      "识别失败：服务出错（HTTP 503）：overloaded",
    );
  });

  it("tells an imported entry and one whose subtitles can carry polished text", () => {
    expect(isImported(entry())).toBe(false);
    expect(isImported(entry({ file: { name: "a.mp4", duration_ms: 1 } }))).toBe(true);
    expect(hasPolishedSegments(entry())).toBe(false);
    expect(hasPolishedSegments(entry({ segments: [{ text: "a", start_ms: 0, end_ms: 1 }] }))).toBe(
      false,
    );
    expect(
      hasPolishedSegments(
        entry({ segments: [{ text: "a", refined: "  ", start_ms: 0, end_ms: 1 }] }),
      ),
    ).toBe(false);
    expect(
      hasPolishedSegments(
        entry({
          segments: [
            { text: "a", start_ms: 0, end_ms: 1 },
            { text: "b", refined: "B。", start_ms: 1, end_ms: 2 },
          ],
        }),
      ),
    ).toBe(true);
  });
});
