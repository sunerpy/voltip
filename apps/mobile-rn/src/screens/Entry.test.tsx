// 纠正识别错误 on the phone (docs/dictation.md §16.9 4, boards 12 and 13) on the whole app with the
// in-memory core: the original text as words (a tap selects, another extends, a tap inside clears,
// a mark is taken whole), the bar, the sheet that opens filled in and without the keyboard, saving
// an entry or a rule and undoing it, the checks before saving, where the fix does not apply, and
// Android's back closing the sheet before it leaves the picking. The phone's engine (Hermes) has no
// `Intl.Segmenter`, so these tests run without it too: Chinese is split one character a word.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { DeviceEventEmitter } from "react-native";
import {
  type DictionaryEntry,
  type HistoryEntry,
  type ProcessState,
  type ReplacementRule,
  wordPieces,
} from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";

import { wordUnits } from "../features/QuickFix";
import { hold, openTab, renderApp, requestIdOf } from "../test/render";

/** The board's sample take (Penpot 「快速纠错」 12): two wordings the polish fixed, one it kept. */
const RAW =
  "下午用维他命测了千问三的流式识别，延迟比上周低了一半，结果发到非书群了，记得把配置同步到酷伯内特斯集群。";
const TEXT =
  "下午用 Voltip 测了千问三的流式识别，延迟比上周低了一半，结果发到非书群了，记得把配置同步到 Kubernetes 集群。";
const T = Date.now();

function take(
  n: number,
  raw: string,
  text: string,
  extra: Partial<HistoryEntry> = {},
): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: T - n * 60_000,
    raw_text: raw,
    text,
    refined: true,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 9400,
    asr_ms: 798,
    refine_ms: 388,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...extra,
  };
}

const SAMPLE = take(1, RAW, TEXT);
const OTHERS = [
  take(2, "今天用维他命跑了一遍回归", "今天用 Voltip 跑了一遍回归。"),
  take(3, "维他命的安装包发给我", "维他命的安装包发给我。"),
  take(4, "结果又发到非书群了", "结果又发到非书群了。"),
];

function dictionaryEntry(term: string, heard: string[]): DictionaryEntry {
  return {
    id: "00000000-0000-4000-9000-000000000001",
    term,
    heard_as: heard,
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: T,
    updated_at_ms: T,
  };
}

const at = (word: string) => RAW.indexOf(word);
const word = (start: number) => screen.getByTestId(`fix-word-${start}`);
/** The status rows' words (each row starts with its icon). */
const statusTexts = (sheet: ReturnType<typeof screen.getByTestId>) =>
  within(sheet).getAllByTestId("fix-status-text");
/** 完成 in the bar. The native stack also hands the bar's actions to its own header, which it
 *  keeps hidden under the app's (navigation.tsx): either one leaves the picking. */
const done = () => {
  const [button] = screen.getAllByTestId("entry-fix-done");
  if (button === undefined) throw new Error("no 完成");
  return button;
};

async function back() {
  await act(async () => {
    DeviceEventEmitter.emit("hardwareBackPress");
    await Promise.resolve();
  });
}

interface Setup {
  history?: HistoryEntry[];
  dictionary?: DictionaryEntry[];
  rules?: ReplacementRule[];
}

/** The app on the phone's in-memory core, at the first entry of 记录. */
async function openEntry(options: Setup = {}) {
  const backend = new MockBackend({
    role: "phone",
    now: () => T,
    history: options.history ?? [SAMPLE, ...OTHERS],
    dictionary: options.dictionary ?? [],
    rules: options.rules ?? [],
  });
  await renderApp({ backend });
  await openTab("history");
  const [first] = await screen.findAllByTestId("phone-history-row");
  if (first === undefined) throw new Error("no row");
  await fireEvent.press(first);
  expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
  return { backend, invoke: jest.spyOn(backend, "invoke") };
}

/** The same, picking words. */
async function openFix(options: Setup = {}) {
  const opened = await openEntry(options);
  await fireEvent.press(screen.getByTestId("entry-fix"));
  expect(await screen.findByTestId("phone-entry-fix")).toBeOnTheScreen();
  return opened;
}

// Hermes, the phone's engine, has no `Intl.Segmenter` (facebook/hermes doc/IntlAPIs.md).
const segmenter = Object.getOwnPropertyDescriptor(Intl, "Segmenter");
beforeAll(() => {
  Reflect.deleteProperty(Intl, "Segmenter");
});
afterAll(() => {
  if (segmenter !== undefined) Object.defineProperty(Intl, "Segmenter", segmenter);
});

describe("纠正识别错误 on the phone", () => {
  it("shows the original as words: a tap selects, another extends, a tap inside clears", async () => {
    await openFix();
    expect(screen.getByText("纠正识别错误")).toBeOnTheScreen();
    expect(done()).toHaveTextContent("完成");
    // The original only: no view switch, no polished text, no actions.
    expect(screen.queryByTestId("phone-entry-text")).toBeNull();
    expect(screen.queryByTestId("entry-fix")).toBeNull();
    expect(screen.getByText("点按词语选择，再点按另一个词语可扩大选择范围。")).toBeOnTheScreen();
    expect(screen.getByTestId("fix-legend")).toHaveTextContent(
      "润色时已改正 2 处· 点按标记可整段选中",
    );
    expect(screen.queryByTestId("fix-bar")).toBeNull();

    await fireEvent.press(word(0));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：下");
    expect(word(0)).toBeSelected();
    await fireEvent.press(word(2));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：下午用");
    expect(word(1)).toBeSelected();
    expect(word(3)).not.toBeSelected();
    // Before the selection: it grows to there.
    await fireEvent.press(word(at("测")));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：下午用维他命测");
    await fireEvent.press(word(1));
    expect(screen.queryByTestId("fix-bar")).toBeNull();
    expect(word(0)).not.toBeSelected();
  });

  it("a mark is taken whole, and the bar says what the polish wrote", async () => {
    await openFix();
    const mark = word(3);
    expect(mark).toHaveAccessibleName("润色时已改正：维他命，润色后为 Voltip");
    expect(within(mark).getByTestId("fix-underline-polish")).toBeOnTheScreen();
    await fireEvent.press(mark);
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：维他命");
    expect(screen.getByText("润色后为「Voltip」")).toBeOnTheScreen();
    expect(screen.getByTestId("fix-bar-dictionary")).toBeEnabled();
    // The selected mark draws its ring instead of the dots.
    expect(within(word(3)).queryByTestId("fix-underline-polish")).toBeNull();
    await fireEvent.press(word(3));
    expect(screen.queryByTestId("fix-bar")).toBeNull();
    // The bar's ✕ clears it too.
    await fireEvent.press(word(at("酷伯内特斯")));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：酷伯内特斯");
    await fireEvent.press(screen.getByTestId("fix-bar-clear"));
    expect(screen.queryByTestId("fix-bar")).toBeNull();
  });

  it("regression: a change a rule made is not marked as the polish's", async () => {
    // Codex review 2026-10-13: with the polish off, a rule's 非书 → 飞书 was marked 「润色时已改正」
    // and the bar said what the polish wrote.
    const feishu: ReplacementRule = {
      id: "00000000-0000-4000-a000-000000000001",
      name: "非书 → 飞书",
      kind: "literal",
      pattern: "非书",
      replacement: "飞书",
      case_sensitive: false,
      enabled: true,
      source: { kind: "manual" },
      pending: false,
      created_at_ms: T,
      updated_at_ms: T,
    };
    const raw = "结果发到非书群了";
    await openFix({
      history: [
        take(1, raw, "结果发到飞书群了", {
          refined: false,
          vocabulary: { corrections: [], rules: [{ id: feishu.id, count: 1 }] },
        }),
      ],
      rules: [feishu],
    });
    expect(screen.queryByTestId("fix-legend")).toBeNull();
    expect(screen.queryAllByTestId("fix-underline-polish")).toEqual([]);
    await fireEvent.press(word(raw.indexOf("非")));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：非");
    expect(screen.queryByText(/润色后为/)).toBeNull();
  });

  describe("regression: a polished take whose rule fired (Codex review 2026-10-13, round 3)", () => {
    // The fired rules are replayed as they are now: a rule changed after the take cannot be, so
    // the take claims nothing for the polish.
    const rule = (updated: number): ReplacementRule => ({
      id: "00000000-0000-4000-a000-000000000002",
      name: "非书 → 飞书",
      kind: "literal",
      pattern: "非书",
      replacement: "飞书",
      case_sensitive: false,
      enabled: true,
      source: { kind: "manual" },
      pending: false,
      created_at_ms: T - 3_600_000,
      updated_at_ms: updated,
    });
    const raw = "结果发到非书群了，用维他命测";
    const polished = take(1, raw, "结果发到飞书群了，用 Voltip 测。", {
      vocabulary: { corrections: [], rules: [{ id: rule(0).id, count: 1 }] },
    });

    it("marks only the polish's own change while the rule is as it fired", async () => {
      await openFix({ history: [polished], rules: [rule(T - 3_600_000)] });
      expect(screen.getByTestId("fix-legend")).toHaveTextContent(
        "润色时已改正 1 处· 点按标记可整段选中",
      );
      expect(word(raw.indexOf("维他命"))).toHaveAccessibleName(
        "润色时已改正：维他命，润色后为 Voltip",
      );
    });

    it("marks nothing for the polish once the rule changed after the take", async () => {
      await openFix({ history: [polished], rules: [rule(T)] });
      expect(screen.queryByTestId("fix-legend")).toBeNull();
      expect(screen.queryAllByTestId("fix-underline-polish")).toEqual([]);
    });
  });

  it("加入词典 opens filled in, without the keyboard, and saves an entry that 撤销 removes", async () => {
    const { backend, invoke } = await openFix();
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    expect(within(sheet).getByRole("header", { name: "加入词典" })).toBeOnTheScreen();
    const heard = within(sheet).getByTestId("fix-heard");
    const term = within(sheet).getByTestId("fix-term");
    expect(heard).toHaveDisplayValue("维他命");
    expect(term).toHaveDisplayValue("Voltip");
    // Filled in: nothing asks for the keyboard.
    expect(heard).not.toHaveProp("autoFocus", true);
    expect(term).not.toHaveProp("autoFocus", true);
    expect(
      within(within(sheet).getByTestId("fix-term-from")).getByText("来自润色结果"),
    ).toBeOnTheScreen();
    expect(within(sheet).getByTestId("fix-note")).toHaveTextContent(
      "识别完成后立即纠正；正确写法还会作为术语发送给云端识别和 AI 润色。",
    );
    // The effect on the recent takes, and the one that kept the words.
    await waitFor(() => {
      expect(statusTexts(sheet)[0]).toHaveTextContent(
        "除本条外，最近 3 条记录中有 2 条含「维他命」。",
      );
    });
    expect(statusTexts(sheet)[1]).toHaveTextContent(
      "其中 1 条的最终文本仍为「维他命」，可能是正确用法。 查看",
    );
    await fireEvent.press(within(sheet).getByTestId("fix-status-show"));
    const examples = within(sheet).getByTestId("fix-examples");
    expect(examples).toHaveTextContent(/今天用维他命跑了一遍回归/);
    expect(examples).toHaveTextContent(/维他命的安装包发给我/);
    expect(within(sheet).getByTestId("fix-status-show")).toHaveTextContent("收起");

    await fireEvent.press(within(sheet).getByTestId("fix-save"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictionary_add", {
        entry: { term: "Voltip", heard_as: ["维他命"], enabled: true },
        historyId: SAMPLE.id,
      });
    });
    await waitFor(() => {
      expect(screen.queryByTestId("fix-sheet-sheet")).toBeNull();
    });
    // The selection is gone and the words are now the dictionary's.
    expect(screen.queryByTestId("fix-bar")).toBeNull();
    await waitFor(() => {
      expect(word(3)).toHaveAccessibleName("已加入词典：维他命，正确写法为 Voltip");
    });
    expect(screen.getByTestId("fix-legend")).toHaveTextContent(/已加入词典 1 处/);
    expect(await screen.findByText("已加入词典 · Voltip")).toBeOnTheScreen();
    const added = (await backend.getState()).dictionary.find((e) => e.term === "Voltip");
    if (added === undefined) throw new Error("not added");
    await fireEvent.press(screen.getByText("撤销"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictionary_remove", { id: added.id });
    });
    await waitFor(() => {
      expect(word(3)).toHaveAccessibleName("润色时已改正：维他命，润色后为 Voltip");
    });
  });

  it("a right spelling the dictionary has takes the words in, and 撤销 puts the entry back", async () => {
    const voltip = dictionaryEntry("Voltip", ["沃尔提普"]);
    const { invoke } = await openFix({ dictionary: [voltip] });
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    expect(within(sheet).getByTestId("fix-save")).toHaveTextContent("添加到「Voltip」");
    await waitFor(() => {
      expect(statusTexts(sheet)[0]).toHaveTextContent(
        "除本条外，最近 3 条记录中有 2 条含「维他命」。",
      );
    });
    await fireEvent.press(within(sheet).getByTestId("fix-save"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictionary_update", {
        id: voltip.id,
        entry: { term: "Voltip", heard_as: ["沃尔提普", "维他命"], enabled: true },
      });
    });
    await fireEvent.press(await screen.findByText("撤销"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("dictionary_update", {
        id: voltip.id,
        entry: { term: "Voltip", heard_as: ["沃尔提普"], enabled: true },
      });
    });
  });

  it("新建规则 finds the words, names itself and previews this entry; 撤销 removes it", async () => {
    const { backend, invoke } = await openFix();
    await fireEvent.press(word(at("非书")));
    await fireEvent.press(word(at("非书") + 1));
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：非书");
    await fireEvent.press(screen.getByTestId("fix-bar-rule"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    expect(within(sheet).getByRole("header", { name: "新建规则" })).toBeOnTheScreen();
    expect(within(sheet).getByTestId("fix-find")).toHaveDisplayValue("非书");
    expect(within(sheet).getByTestId("fix-replace")).toHaveDisplayValue("");
    await fireEvent.changeText(within(sheet).getByTestId("fix-replace"), "飞书");
    expect(within(sheet).getByTestId("fix-name")).toHaveDisplayValue("非书 → 飞书");
    await waitFor(() => {
      expect(within(sheet).getByTestId("fix-preview")).toHaveTextContent(/结果发到非飞书群了/);
    });
    await waitFor(() => {
      expect(statusTexts(sheet)[0]).toHaveTextContent(
        "除本条外，最近 3 条记录中有 1 条会被改动。 查看",
      );
    });
    expect(within(sheet).getByTestId("fix-note")).toHaveTextContent(
      "替换规则在 AI 润色之后执行，按规则列表的顺序改写最终文本。新规则排在列表末尾。",
    );
    // A regex and the case switch are the rule's to set.
    await fireEvent.press(within(sheet).getByTestId("fix-case"));
    await fireEvent.press(within(sheet).getByTestId("fix-case"));
    await fireEvent.press(within(sheet).getByTestId("fix-save"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("rules_add", {
        rule: {
          name: "非书 → 飞书",
          kind: "literal",
          pattern: "非书",
          replacement: "飞书",
          case_sensitive: true,
          enabled: true,
        },
      });
    });
    expect(await screen.findByText("已新建规则 · 非书 → 飞书")).toBeOnTheScreen();
    const added = (await backend.getState()).rules.find((r) => r.name === "非书 → 飞书");
    if (added === undefined) throw new Error("not added");
    await fireEvent.press(screen.getByText("撤销"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("rules_remove", { id: added.id });
    });
  });

  it("switching to 替换规则 and back carries the words, and a refused regex says why", async () => {
    const { invoke } = await openFix();
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    await fireEvent.press(within(sheet).getByTestId("fix-kind-rule"));
    expect(within(sheet).getByTestId("fix-find")).toHaveDisplayValue("维他命");
    expect(within(sheet).getByTestId("fix-replace")).toHaveDisplayValue("Voltip");
    await fireEvent.changeText(within(sheet).getByTestId("fix-find"), "维他(命");
    await fireEvent.press(within(sheet).getByTestId("fix-rule-regex"));
    await waitFor(() => {
      expect(within(sheet).getByTestId("fix-status")).toHaveTextContent(/正则无法编译/);
    });
    await waitFor(() => {
      expect(within(sheet).getByTestId("fix-save")).toBeDisabled();
    });
    await fireEvent.press(within(sheet).getByTestId("fix-rule-literal"));
    await fireEvent.press(within(sheet).getByTestId("fix-kind-dictionary"));
    expect(within(sheet).getByTestId("fix-heard")).toHaveDisplayValue("维他(命");
    expect(within(sheet).getByTestId("fix-term")).toHaveDisplayValue("Voltip");
    // 取消 closes without a word sent.
    await fireEvent.press(within(sheet).getByTestId("fix-cancel"));
    await waitFor(() => {
      expect(screen.queryByTestId("fix-sheet-sheet")).toBeNull();
    });
    expect(invoke).not.toHaveBeenCalledWith("dictionary_add", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("rules_add", expect.anything());
  });

  it("a form another entry has is refused before saving, with the way to the dictionary", async () => {
    // 维生素's form is a mark of its own: taken whole, as the dictionary corrects it.
    await openFix({ dictionary: [dictionaryEntry("维生素", ["维他命"])] });
    expect(word(3)).toHaveAccessibleName("已加入词典：维他命，正确写法为 维生素");
    expect(screen.getByTestId("fix-legend")).toHaveTextContent(
      "润色时已改正 1 处已加入词典 1 处· 点按标记可整段选中",
    );
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    expect(statusTexts(sheet)).toHaveLength(1);
    expect(statusTexts(sheet)[0]).toHaveTextContent(
      "「维他命」已是词条「维生素」的曾听成写法。 打开词典",
    );
    expect(within(sheet).getByTestId("fix-save")).toBeDisabled();
    await fireEvent.press(within(sheet).getByTestId("fix-status-openDictionary"));
    expect(await screen.findByTestId("phone-dictionary")).toBeOnTheScreen();
    await waitFor(() => {
      expect(screen.queryByTestId("fix-sheet-sheet")).toBeNull();
    });
  });

  it("the core's refusal keeps the sheet open with the reason under the field", async () => {
    const { backend } = await openFix();
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    jest
      .spyOn(backend, "invoke")
      .mockRejectedValueOnce(new Error("dictionary: 误识别写法不能包含换行或控制字符"));
    await fireEvent.press(within(sheet).getByTestId("fix-save"));
    // The core's words without its prefix (`errorText`).
    expect(await within(sheet).findByText("误识别写法不能包含换行或控制字符")).toBeOnTheScreen();
    expect(screen.getByTestId("fix-sheet-sheet")).toBeOnTheScreen();
    // Editing clears it; the save is offered again.
    await fireEvent.changeText(within(sheet).getByTestId("fix-term"), "Voltip!");
    expect(within(sheet).queryByText("误识别写法不能包含换行或控制字符")).toBeNull();
    expect(within(sheet).queryByTestId("fix-term-from")).toBeNull();
  });

  it("words that cannot be a dictionary form make a rule, and the bar says why", async () => {
    const long = "测".repeat(70);
    await openFix({ history: [take(1, long, long, { refined: false })] });
    await fireEvent.press(word(0));
    await fireEvent.press(word(69));
    expect(screen.getByTestId("fix-bar-block")).toHaveTextContent("词典写法最多 64 个字符");
    expect(screen.getByTestId("fix-bar-dictionary")).toBeDisabled();
    await fireEvent.press(screen.getByTestId("fix-bar-rule"));
    const sheet = await screen.findByTestId("fix-sheet-sheet");
    expect(within(sheet).getByTestId("fix-find")).toHaveDisplayValue(long);
    // No marks without a polish or a dictionary: no legend.
    expect(screen.queryByTestId("fix-legend")).toBeNull();
  });

  it("Android back closes the sheet, then leaves the picking, then the page", async () => {
    await openFix();
    await fireEvent.press(word(3));
    await fireEvent.press(screen.getByTestId("fix-bar-dictionary"));
    expect(await screen.findByTestId("fix-sheet-sheet")).toBeOnTheScreen();
    await back();
    await waitFor(() => {
      expect(screen.queryByTestId("fix-sheet-sheet")).toBeNull();
    });
    expect(screen.getByTestId("phone-entry-fix")).toBeOnTheScreen();
    expect(screen.getByTestId("fix-bar-text")).toHaveTextContent("已选择：维他命");
    await back();
    expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
    expect(screen.getByTestId("entry-fix")).toBeOnTheScreen();
    expect(screen.queryAllByTestId("entry-fix-done")).toEqual([]);
    await back();
    expect(await screen.findByTestId("phone-history")).toBeOnTheScreen();
  });

  it("the bar's back and 完成 leave the picking and stay on the entry", async () => {
    await openFix();
    await fireEvent.press(screen.getByTestId("header-back"));
    expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("entry-fix"));
    expect(await screen.findByTestId("phone-entry-fix")).toBeOnTheScreen();
    await fireEvent.press(word(0));
    await fireEvent.press(done());
    expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
    // The polished view is back, and a new picking starts with nothing selected.
    expect(screen.getByTestId("phone-entry-text")).toHaveTextContent(TEXT);
    await fireEvent.press(screen.getByTestId("entry-fix"));
    expect(await screen.findByTestId("phone-entry-fix")).toBeOnTheScreen();
    expect(screen.queryByTestId("fix-bar")).toBeNull();
  });

  it("a take the computer recognised says where to fix it instead", async () => {
    await openEntry({
      history: [take(1, RAW, TEXT, { origin: { kind: "sent", device: "MacBook Pro" } })],
    });
    expect(screen.getByTestId("entry-fix-note")).toHaveTextContent(
      "这条在电脑上识别，请在电脑的历史记录中纠正。",
    );
    expect(screen.queryByTestId("entry-fix")).toBeNull();
  });

  it("a text the phone sent, or a take with no words, offers no picking", async () => {
    await openEntry({
      history: [
        take(1, "x", "x", { origin: { kind: "typed", device: "MacBook Pro" } }),
        take(2, " ", "", { refined: false, outcome: { kind: "failed", reason: "empty" } }),
      ],
    });
    expect(screen.queryByTestId("entry-fix")).toBeNull();
    expect(screen.queryByTestId("entry-fix-note")).toBeNull();
    await fireEvent.press(screen.getByTestId("header-back"));
    const rows = await screen.findAllByTestId("phone-history-row");
    const second = rows[1];
    if (second === undefined) throw new Error("no second row");
    await fireEvent.press(second);
    expect(await screen.findByTestId("phone-entry")).toBeOnTheScreen();
    expect(screen.queryByTestId("entry-fix")).toBeNull();
  });
});

describe("a take whose polish failed", () => {
  it("regression: says why in the interface's words and keeps the service's own (docs/dictation.md §3.12)", async () => {
    // 2026-10-25: the row showed the client's English words, 「refine service overloaded (HTTP 503)」.
    await openEntry({
      history: [
        take(1, RAW, RAW, {
          refined: false,
          refine_ms: undefined,
          refine_failure: "rate_limited",
          refine_error:
            "服务繁忙：refine service overloaded (HTTP 503): This model is currently experiencing high demand.",
        }),
      ],
    });
    expect(screen.getByText("润色失败原因")).toBeOnTheScreen();
    expect(
      screen.getByText("服务繁忙：This model is currently experiencing high demand."),
    ).toBeOnTheScreen();
    expect(screen.queryByText(/refine service/)).toBeNull();
  });
});

/** The app on `backend`, at the first entry of 记录 (or where its row leads). */
async function openOn(backend: MockBackend) {
  await renderApp({ backend });
  await openTab("history");
  const [first] = await screen.findAllByTestId("phone-history-row");
  if (first === undefined) throw new Error("no row");
  await fireEvent.press(first);
}

const chars = (text: string) => Array.from(text).length;

// 记录详情 itself: the texts, the facts, copy, share, star and delete, and the long entry's tools.
describe("the entry page", () => {
  const PROCESSED = "要点：流式识别的延迟降低了一半。";

  it("shows the polished, original and processed texts with their facts; copies, shares and stars", async () => {
    const entry = take(1, RAW, TEXT, {
      processed: { text: PROCESSED, preset: { id: "notes", name: "要点纪要" }, at_ms: T },
      preset: { id: "proofread", name: "校对" },
      scene: { id: "s-chat", name: "chat", builtin: "chat" },
    });
    const { backend } = await openEntry({ history: [entry] });
    const text = () => screen.getByTestId("phone-entry-text");
    expect(text()).toHaveTextContent(TEXT);
    expect(screen.getByTestId("entry-outcome")).toHaveTextContent("已插入 · 粘贴");
    await fireEvent.press(screen.getByText("原文"));
    expect(text()).toHaveTextContent(RAW);
    await fireEvent.press(screen.getByText("处理后"));
    expect(text()).toHaveTextContent(PROCESSED);
    expect(screen.getByText("由「要点纪要」处理 · 原文保留")).toBeOnTheScreen();
    for (const fact of ["Qwen/Qwen3-ASR-1.7B", "校对", "即时聊天", "总计 1,186 ms"]) {
      expect(screen.getByText(fact)).toBeOnTheScreen();
    }

    // What is copied and shared is the view on screen.
    await fireEvent.press(screen.getByText("复制"));
    expect(await screen.findByText(`已复制到剪贴板 · ${chars(PROCESSED)} 字`)).toBeOnTheScreen();
    expect(backend.phoneClipboard).toBe(PROCESSED);
    await fireEvent.press(screen.getByText("分享"));
    await waitFor(() => {
      expect(backend.shared).toEqual([PROCESSED]);
    });
    await fireEvent.press(screen.getByText("收藏"));
    expect(await screen.findByText("取消收藏")).toBeOnTheScreen();
    expect((await backend.historyEntry(entry.id))?.starred).toBe(true);
    await fireEvent.press(screen.getByText("取消收藏"));
    expect(await screen.findByText("收藏")).toBeOnTheScreen();
    expect((await backend.historyEntry(entry.id))?.starred).toBe(false);
  });

  it("deletes after asking and goes back to 记录; says why a copy, a share, a star or a deletion did not go through", async () => {
    const { backend, invoke } = await openEntry();
    const paste = jest.spyOn(backend, "pasteText");
    paste.mockResolvedValueOnce({ kind: "failed", reason: "invalid" });
    await fireEvent.press(screen.getByText("复制"));
    expect(await screen.findByText("复制失败")).toBeOnTheScreen();
    paste.mockRejectedValueOnce(new Error("剪贴板不可用"));
    await fireEvent.press(screen.getByText("复制"));
    await waitFor(() => {
      expect(paste).toHaveBeenCalledTimes(2);
    });
    expect(await screen.findByText("复制失败")).toBeOnTheScreen();
    paste.mockRestore();
    // One reason for each refusal below, so each toast replaces the last.
    const refuse = () => invoke.mockRejectedValueOnce(new Error("核心没有响应"));
    refuse();
    await fireEvent.press(screen.getByText("分享"));
    expect(await screen.findByText(/核心没有响应/)).toBeOnTheScreen();
    refuse();
    await fireEvent.press(screen.getByText("收藏"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("history_star", { id: SAMPLE.id, starred: true });
    });
    expect(screen.getByText("收藏")).toBeOnTheScreen();

    await fireEvent.press(screen.getByText("删除"));
    expect(await screen.findByText("删除这条记录？")).toBeOnTheScreen();
    expect(
      screen.getByText(new RegExp(`${TEXT.slice(0, 40)}…。此操作无法撤销。$`)),
    ).toBeOnTheScreen();
    refuse();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("history_delete", { id: SAMPLE.id });
    });
    expect(screen.getByTestId("phone-entry")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("删除"));
    await fireEvent.press(await screen.findByTestId("shell-confirm"));
    expect(await screen.findByTestId("phone-history")).toBeOnTheScreen();
    await waitFor(() => {
      expect(screen.getAllByTestId("phone-history-row")).toHaveLength(OTHERS.length);
    });
  });

  it("processes a long entry with a chosen preset, cancels a run, and shares it as subtitles or text", async () => {
    const long = take(1, RAW, RAW, {
      refined: false,
      refine_ms: undefined,
      duration_ms: 180_000,
      segments: [{ text: "下午用维他命测了千问三的流式识别。", start_ms: 0, end_ms: 4000 }],
    });
    const { backend } = await openEntry({ history: [long] });
    // The core's answers are the test's: each step stays on the screen until the next one.
    const invoke = hold(backend, "history_process", "history_process_cancel");
    const answer = async (state: ProcessState) => {
      const requestId = requestIdOf(invoke, "history_process");
      await act(async () => {
        backend.publish({
          type: "history_process",
          request_id: requestId,
          id: long.id,
          state,
        });
        await Promise.resolve();
      });
    };
    const tools = within(screen.getByTestId("phone-entry-long"));
    expect(tools.queryByText("导出文本时使用处理后文本。")).toBeNull();
    await fireEvent.press(tools.getByLabelText("预设"));
    await fireEvent.press(await screen.findByRole("radio", { name: "要点纪要" }));
    await fireEvent.press(tools.getByText("开始处理"));
    expect(invoke).toHaveBeenCalledWith("history_process", {
      requestId: expect.any(Number),
      id: long.id,
      preset: "notes",
    });
    await answer({ state: "running", done: 1, total: 2 });
    expect(tools.getByText("已处理 1/2 部分")).toBeOnTheScreen();
    await answer({
      state: "done",
      processed: { text: "要点：流式识别。", preset: { id: "notes", name: "要点纪要" }, at_ms: T },
    });
    expect(await tools.findByText("重新处理")).toBeOnTheScreen();
    expect(tools.getByText("导出文本时使用处理后文本。")).toBeOnTheScreen();

    await fireEvent.press(tools.getByText("重新处理"));
    await fireEvent.press(await tools.findByText("取消"));
    expect(invoke).toHaveBeenCalledWith("history_process_cancel", {
      requestId: expect.any(Number),
    });
    await answer({ state: "cancelled" });
    expect(await tools.findByText("已取消，未保存结果。")).toBeOnTheScreen();

    await fireEvent.press(tools.getByText("分享字幕（SRT）"));
    await fireEvent.press(tools.getByText("分享文本（TXT）"));
    await waitFor(() => {
      expect(backend.exports.map((e) => e.format)).toEqual(["srt", "txt"]);
    });
    expect(backend.exports[0]?.fileName).toMatch(/^Voltip \d{4}-\d{2}-\d{2} \d{2}\.\d{2}$/);
  });

  it("says why a long entry could not be processed or shared, and offers no subtitles without segments", async () => {
    const long = take(1, RAW, RAW, { refined: false, refine_ms: undefined, duration_ms: 180_000 });
    const backend = new MockBackend({ role: "phone", now: () => T, history: [long], builtIn: {} });
    await openOn(backend);
    const tools = within(await screen.findByTestId("phone-entry-long"));
    expect(tools.getByText("分享字幕（SRT）")).toBeDisabled();
    await fireEvent.press(tools.getByText("开始处理"));
    expect(
      await tools.findByText("处理失败：尚未配置 AI 润色服务，无法用预设处理"),
    ).toBeOnTheScreen();
    jest
      .spyOn(backend, "historyExport")
      .mockResolvedValueOnce({ kind: "failed", code: "write", detail: "磁盘已满" });
    await fireEvent.press(tools.getByText("分享文本（TXT）"));
    expect(await screen.findByText("文件未能保存：磁盘已满")).toBeOnTheScreen();
  });

  it("a take sent to a computer names it and leaves the details to the computer; one too large to upload says so", async () => {
    const sent = take(1, "会议改到三点", "会议改到三点。", {
      origin: { kind: "sent", device: "Studio" },
    });
    const { backend } = await openEntry({ history: [sent] });
    expect(screen.getByText("发送到 Studio")).toBeOnTheScreen();
    expect(screen.getByText("识别和润色的详细信息保存在电脑的记录中。")).toBeOnTheScreen();
    expect(screen.queryByText("识别模型")).toBeNull();
    expect(screen.queryByText("这条记录太大，没有上传到电脑。")).toBeNull();
    await act(async () => {
      backend.simulateTooLarge([sent.id]);
      await Promise.resolve();
    });
    expect(await screen.findByText("这条记录太大，没有上传到电脑。")).toBeOnTheScreen();
  });

  it("an entry deleted elsewhere shows as gone", async () => {
    const { backend } = await openEntry();
    await act(async () => {
      await backend.invoke("history_delete", { id: SAMPLE.id });
    });
    expect(await screen.findByText("这条记录已删除。")).toBeOnTheScreen();
    expect(screen.getByText("这条记录已被删除或清空。")).toBeOnTheScreen();
  });

  it("an entry the core cannot read shows as gone", async () => {
    const backend = new MockBackend({ role: "phone", now: () => T, history: [SAMPLE] });
    jest.spyOn(backend, "historyEntry").mockRejectedValue(new Error("读不出来"));
    await openOn(backend);
    expect(await screen.findByText("这条记录已删除。")).toBeOnTheScreen();
  });
});

describe("the words' layout (wordUnits)", () => {
  it("sticks punctuation to its word, an opening bracket to the next, and breaks lines", () => {
    const text = "他说「你好」，然后 OK. (再见)\n下一行";
    const units = wordUnits(wordPieces(text, [], null));
    const shown = units.map((u) =>
      u.kind === "break" ? "⏎" : `${u.pieces.map((p) => p.text).join("")}${u.word ? "" : "∅"}`,
    );
    expect(shown).toEqual([
      "他",
      "说",
      "「你",
      "好」，",
      "然",
      "后",
      "OK.",
      "(再",
      "见)",
      "⏎",
      "下",
      "一",
      "行",
    ]);
    // Punctuation with no word before it (after a space, or at the start) stands alone.
    expect(
      wordUnits(wordPieces("— 好", [], null)).map((u) =>
        u.kind === "unit" ? [u.pieces.map((p) => p.text).join(""), u.word?.text] : ["⏎"],
      ),
    ).toEqual([
      ["—", undefined],
      ["好", "好"],
    ]);
    expect(
      wordUnits(wordPieces("「」好", [], null)).map((u) =>
        u.kind === "unit" ? u.pieces.map((p) => p.text).join("") : "⏎",
      ),
    ).toEqual(["「」", "好"]);
  });
});
