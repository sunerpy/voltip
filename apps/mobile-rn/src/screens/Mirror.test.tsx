// A computer's copy on the phone (docs/dictation.md §20.8): 记录's chips switch to it, its rows open
// read-only entries, and 设置 lists the computer's settings as the copy last received them.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type HistoryEntry, MAX_PASTE_TEXT_CHARS, defaultSettings } from "@voltip/shared";
import { MockBackend, type MockMirror, mockMirrorProfile } from "@voltip/shared/mock";

import { nth, openTab, renderApp } from "../test/render";
import { appTheme } from "../theme/themes";

const DESKTOP = "ab".repeat(32);

function entry(n: number, over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: Date.now() - n * 60_000,
    raw_text: "今天下午三点开会",
    text: "今天下午三点开会。",
    refined: false,
    asr_model: "whisper-large-v3",
    duration_ms: 4_200,
    asr_ms: 320,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...over,
  };
}

function mirror(history: HistoryEntry[], over: Partial<MockMirror> = {}): MockMirror {
  return {
    view: {
      desktop: DESKTOP,
      name: "办公室电脑",
      state: "up_to_date",
      entries: history.length,
      synced_at_ms: Date.now() - 120_000,
    },
    history,
    profile: mockMirrorProfile(),
    ...over,
  };
}

const REFINED = entry(1, {
  refined: true,
  refine_model: "gpt-oss-120b",
  refine_ms: 900,
  preset: { id: "proofread", name: "校对" },
  processed: {
    text: "会议：今天下午三点。",
    preset: { id: "11111111-1111-4111-8111-111111111111", name: "会议纪要" },
    at_ms: Date.now(),
  },
  origin: { kind: "take", device: "小米手机" },
  app: { id: "com.tencent.wework", name: "企业微信" },
  scene: { id: "s-chat", name: "chat", builtin: "chat" },
});

/** 记录 › the computer's chip; answers the copy's state line. */
async function openCopy() {
  await openTab("history");
  await fireEvent.press(await screen.findByText("办公室电脑"));
  return screen.findByTestId("phone-history-mirror-state");
}

async function openEntry(index = 0) {
  await openCopy();
  const rows = await screen.findAllByTestId("phone-history-row");
  await fireEvent.press(nth(rows, index));
  return screen.findByTestId("phone-mirror-entry");
}

/** A desktop's lists, made by its own core, as a copy would carry them. */
async function computerLists() {
  const pc = new MockBackend({ role: "desktop" });
  await pc.invoke("presets_add", { preset: { name: "周报", prompt: "整理成周报。" } });
  await pc.invoke("dictionary_add", {
    entry: { term: "Voltip", heard_as: ["沃尔提普", "伏特"], enabled: true },
  });
  await pc.invoke("dictionary_add", { entry: { term: "飞书", heard_as: [], enabled: true } });
  await pc.invoke("rules_add", {
    rule: {
      name: "去掉嗯",
      kind: "literal",
      pattern: "嗯",
      replacement: "",
      case_sensitive: true,
      enabled: true,
    },
  });
  const { presets, dictionary, rules, scenes } = pc.peek();
  pc.destroy();
  return { presets, dictionary, rules, scenes };
}

describe("记录 › a computer's copy", () => {
  it("switches between this phone and the computer: the copy's state, its rows and no counts", async () => {
    await renderApp({ mock: { mirrors: [mirror([REFINED, entry(2)])] } });
    await openTab("history");
    const source = await screen.findByLabelText("记录来源");
    expect(within(source).getByText("这部手机")).toBeOnTheScreen();
    expect(screen.getByTestId("phone-history-stats")).toBeOnTheScreen();
    const state = await openCopy();
    expect(state).toHaveTextContent(/^已同步 2 条 · /);
    expect(screen.queryByTestId("phone-history-stats")).toBeNull();
    expect(await screen.findAllByTestId("phone-history-row")).toHaveLength(2);
    // The row says which phone the take came from and the scene it ran with.
    expect(screen.getByText("手机 · 小米手机")).toBeOnTheScreen();
    expect(screen.getByText("即时聊天")).toBeOnTheScreen();
    await fireEvent.press(within(source).getByText("这部手机"));
    expect(await screen.findByTestId("phone-history-stats")).toBeOnTheScreen();
    expect(screen.queryByTestId("phone-history-mirror-state")).toBeNull();
  });

  it("says when a computer's copy holds no entries yet", async () => {
    await renderApp({
      mock: { mirrors: [mirror([], { view: { ...mirror([]).view, state: "syncing" } })] },
    });
    expect(await openCopy()).toHaveTextContent("正在同步 · 已收到 0 条");
    expect(screen.getByText("尚无记录")).toBeOnTheScreen();
    expect(
      screen.getByText("这台电脑的历史记录同步后会出现在这里，手机上只能查看。"),
    ).toBeOnTheScreen();
  });

  it("opens an entry read-only with its three texts and its facts; copy and share say so", async () => {
    const { backend } = await renderApp({ mock: { mirrors: [mirror([REFINED, entry(2)])] } });
    const page = within(await openEntry());
    const text = () => page.getByTestId("phone-mirror-entry-text");
    expect(text()).toHaveTextContent("今天下午三点开会。");
    await fireEvent.press(page.getByText("原文"));
    expect(text()).toHaveTextContent("今天下午三点开会");
    await fireEvent.press(page.getByText("处理后"));
    expect(text()).toHaveTextContent("会议：今天下午三点。");
    expect(page.getByText("由「会议纪要」处理 · 原文保留")).toBeOnTheScreen();
    expect(page.queryByTestId("phone-mirror-entry-shortened")).toBeNull();
    expect(page.getByText("电脑上的记录在手机上只能查看，不能收藏或删除。")).toBeOnTheScreen();
    for (const fact of [
      "手机 · 小米手机",
      "whisper-large-v3",
      "gpt-oss-120b",
      "校对",
      "企业微信",
      "即时聊天",
      "总计 1,220 ms",
    ]) {
      expect(page.getByText(fact)).toBeOnTheScreen();
    }

    // What is copied and shared is the view on screen.
    await fireEvent.press(page.getByText("复制"));
    expect(await screen.findByText("已复制到剪贴板 · 10 字")).toBeOnTheScreen();
    expect(backend.phoneClipboard).toBe("会议：今天下午三点。");
    await fireEvent.press(page.getByText("分享"));
    await waitFor(() => {
      expect(backend.shared).toEqual(["会议：今天下午三点。"]);
    });
  });

  it("an entry not cleaned up says so; one that arrived shortened or past the paste limit says why it cannot go whole", async () => {
    const long = "长".repeat(MAX_PASTE_TEXT_CHARS + 1);
    const plain = entry(1, { text: long, raw_text: long });
    await renderApp({
      mock: { mirrors: [mirror([plain], { shortened: [plain.id] })] },
    });
    const page = within(await openEntry());
    expect(page.queryByText("原文")).toBeNull();
    expect(page.getByTestId("phone-mirror-entry-shortened")).toHaveTextContent(
      "这条记录太长，手机上只显示了一部分，完整内容请在电脑上查看。",
    );
    expect(page.getByText("未润色")).toBeOnTheScreen();
    expect(page.getByText("超过 5 万字，无法整段复制或分享。")).toBeOnTheScreen();
    expect(page.getByText("复制")).toBeDisabled();
    expect(page.getByText("分享")).toBeDisabled();
  });

  it("says when copying or sharing failed, and that an entry the computer deleted is gone", async () => {
    const kept = entry(2);
    const { backend } = await renderApp({ mock: { mirrors: [mirror([REFINED, kept])] } });
    const page = within(await openEntry());
    const paste = jest.spyOn(backend, "pasteText");
    paste.mockResolvedValueOnce({ kind: "failed", reason: "invalid" });
    await fireEvent.press(page.getByText("复制"));
    expect(await screen.findByText("复制失败")).toBeOnTheScreen();
    paste.mockRejectedValueOnce(new Error("剪贴板不可用"));
    await fireEvent.press(page.getByText("复制"));
    await waitFor(() => {
      expect(paste).toHaveBeenCalledTimes(2);
    });
    expect(await screen.findByText("复制失败")).toBeOnTheScreen();
    paste.mockRestore();
    jest.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("无法打开分享"));
    await fireEvent.press(page.getByText("分享"));
    expect(await screen.findByText(/无法打开分享/)).toBeOnTheScreen();

    // The computer deleted the entry: the next batch no longer has it.
    await act(async () => {
      backend.simulateMirror(mirror([kept]));
    });
    expect(await screen.findByText("这条记录已删除。")).toBeOnTheScreen();
    expect(screen.getByText("这条记录已被删除或清空。")).toBeOnTheScreen();
  });

  // Regression (2026-10-28): the label's `warn` tone had no lamp of its own, so a take whose paste
  // fell back to the clipboard showed the grey lamp of a take with nothing to say.
  it("regression: a take that fell back to the clipboard shows the warning lamp", async () => {
    const fallback = entry(1, { outcome: { kind: "clipboard", reason: "目标窗口没有焦点" } });
    await renderApp({ mock: { mirrors: [mirror([fallback])] } });
    const page = within(await openEntry());
    expect(page.getByTestId("entry-outcome")).toHaveTextContent("已复制到剪贴板");
    expect(page.getByTestId("entry-outcome-lamp")).toHaveStyle({
      backgroundColor: appTheme("glass").voltip.warning,
    });
  });

  it("an entry that cannot be read shows as gone", async () => {
    const { backend } = await renderApp({ mock: { mirrors: [mirror([REFINED])] } });
    await openCopy();
    jest.spyOn(backend, "mirrorHistoryEntry").mockRejectedValueOnce(new Error("副本损坏"));
    await fireEvent.press(await screen.findByTestId("phone-history-row"));
    expect(await screen.findByText("这条记录已删除。")).toBeOnTheScreen();
  });
});

describe("设置 › a computer's settings", () => {
  async function openSettings() {
    await openTab("settings");
    await fireEvent.press(await screen.findByTestId(`settings-computerSettings-${DESKTOP}`));
    return screen.findByTestId("phone-computer-settings");
  }

  it("lists the computer's look, speech, polish and lists as the copy last received them", async () => {
    const lists = await computerLists();
    const settings = {
      ...defaultSettings(),
      locale: "en" as const,
      theme: "warm" as const,
      follow_system_theme: false,
    };
    await renderApp({
      mock: { mirrors: [mirror([entry(1)], { profile: mockMirrorProfile(settings, lists) })] },
    });
    await openTab("settings");
    expect(await screen.findByText("办公室电脑 的设置")).toBeOnTheScreen();
    const page = within(await openSettings());
    expect(
      page.getByText("办公室电脑 的设置，在手机上只能查看；在电脑上修改后会同步到手机。"),
    ).toBeOnTheScreen();
    expect(page.getByText(/^已同步 1 条 · /)).toBeOnTheScreen();
    for (const fact of [
      "English",
      "暖纸",
      "内置服务 · Qwen/Qwen3-ASR-1.7B",
      "已开启",
      "内置服务 · qwen/qwen3.8-27b",
      "校对",
      "周报",
      "整理成周报。",
      "Voltip",
      "听成：沃尔提普、伏特",
      "飞书",
      "去掉嗯",
      "嗯 → ",
      "即时聊天",
    ]) {
      expect(page.getByText(fact)).toBeOnTheScreen();
    }
    expect(page.queryByText("无")).toBeNull();
  });

  it("says when no settings arrived, then shows the ones that do: a theme following the system, no polish, empty lists", async () => {
    const { backend } = await renderApp({
      mock: { mirrors: [mirror([entry(1)], { profile: null })] },
    });
    let page = within(await openSettings());
    expect(page.getByText("尚未收到这台电脑的设置。")).toBeOnTheScreen();
    const engines = { ...defaultSettings().engines, refine_enabled: false };
    const profile = {
      ...mockMirrorProfile({ ...defaultSettings(), follow_system_theme: true, engines }),
      asr_model: "",
      refine_model: "",
    };
    delete profile.llm_provider;
    await act(async () => {
      backend.simulateMirror(
        mirror([entry(1)], {
          view: { ...mirror([]).view, entries: 1, synced_at_ms: Date.now() },
          profile,
        }),
      );
    });
    expect(await screen.findByText("未开启")).toBeOnTheScreen();
    page = within(screen.getByTestId("phone-computer-settings"));
    // Both the language (set to follow the system) and the theme.
    expect(page.getAllByText("跟随系统")).toHaveLength(2);
    expect(page.getByText("内置服务 · —")).toBeOnTheScreen();
    expect(page.getByText("— · —")).toBeOnTheScreen();
    expect(page.getAllByText("无")).toHaveLength(4);
  });

  it("settings that cannot be read show as not received", async () => {
    const { backend } = await renderApp({ mock: { mirrors: [mirror([entry(1)])] } });
    jest.spyOn(backend, "mirrorProfile").mockRejectedValueOnce(new Error("副本损坏"));
    const page = within(await openSettings());
    expect(page.getByText("尚未收到这台电脑的设置。")).toBeOnTheScreen();
  });
});
