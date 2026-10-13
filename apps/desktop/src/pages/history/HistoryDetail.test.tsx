// The history detail's quick fix (docs/dictation.md §16.9) on the whole app with the in-memory
// core: marks, the selection toolbar, the dictionary and rule cards, the diff view's question,
// the undo toast, where the fix does not apply, and the keyboard.
import { type DictionaryEntry, type HistoryEntry, type ReplacementRule } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";

/** The board's sample take (Penpot 「快速纠错」 01): two wordings the polish fixed, one it kept. */
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

let seq = 0;
function entry(
  term: string,
  heard: string[],
  extra: Partial<DictionaryEntry> = {},
): DictionaryEntry {
  seq += 1;
  return {
    id: `00000000-0000-4000-9000-${String(seq).padStart(12, "0")}`,
    term,
    heard_as: heard,
    enabled: true,
    source: { kind: "manual" },
    created_at_ms: T,
    updated_at_ms: T,
    ...extra,
  };
}

function setup(
  options: {
    history?: HistoryEntry[];
    dictionary?: DictionaryEntry[];
    rules?: ReplacementRule[];
  } = {},
) {
  const backend = new MockBackend({
    now: () => T,
    history: options.history ?? [SAMPLE, ...OTHERS],
    dictionary: options.dictionary ?? [],
    rules: options.rules ?? [],
  });
  const user = userEvent.setup();
  const view = renderApp({ path: "/history", backend });
  return { ...view, backend, user };
}

async function showRaw(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("radio", { name: "原文" }));
  return screen.getByTestId("entry-text");
}

/** Select `word` (its `nth` occurrence) in the detail's text the way a drag does, and let go. */
function selectWord(word: string, nth = 0): void {
  const root = screen.getByTestId("entry-text").querySelector("[data-fix-text]");
  if (root === null) throw new Error("no fix text");
  const leaves = [...root.querySelectorAll("[data-at]")];
  let seen = 0;
  for (const leaf of leaves) {
    const text = leaf.textContent ?? "";
    let at = text.indexOf(word);
    while (at >= 0) {
      if (seen === nth) {
        const node = leaf.firstChild;
        if (node === null) throw new Error("empty leaf");
        const range = document.createRange();
        range.setStart(node, at);
        range.setEnd(node, at + word.length);
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        fireEvent.mouseUp(root);
        return;
      }
      seen += 1;
      at = text.indexOf(word, at + 1);
    }
  }
  throw new Error(`"${word}" is not in the text`);
}

function card() {
  return screen.getByTestId("fix-card");
}

describe("History detail · quick fix (docs/dictation.md section 16.9)", () => {
  it("marks the polish's replacements in the original text, with a legend, labels and a tooltip", async () => {
    const { user } = setup();
    expect(screen.queryAllByTestId("fix-mark")).toEqual([]);
    await showRaw(user);
    const marks = screen.getAllByTestId("fix-mark");
    expect(marks.map((m) => m.getAttribute("aria-label"))).toEqual([
      "润色时已改正：维他命，润色后为 Voltip",
      "润色时已改正：酷伯内特斯，润色后为 Kubernetes",
    ]);
    expect(marks.map((m) => m.dataset["mark"])).toEqual(["polish", "polish"]);
    expect(screen.getByTestId("fix-legend")).toHaveTextContent(
      "润色时已改正 2 处· 点按标记可加入词典",
    );
    await user.hover(marks[0] ?? document.body);
    expect(screen.getByTestId("fix-tip")).toHaveTextContent("润色后为「Voltip」· 点按加入词典");
    await user.unhover(marks[0] ?? document.body);
    expect(screen.queryByTestId("fix-tip")).toBeNull();
    // Marks are buttons after the view switch in the Tab order; the focus shows the tooltip too.
    act(() => {
      marks[0]?.focus();
    });
    expect(screen.getByTestId("fix-tip")).toBeInTheDocument();
    act(() => {
      marks[0]?.blur();
    });
    expect(screen.queryByTestId("fix-tip")).toBeNull();
  });

  it("a mark opens the dictionary card pre-filled from the polished text, says what it would change and saves with the row's id", async () => {
    const { user, backend } = setup();
    await showRaw(user);
    const [mark] = screen.getAllByTestId("fix-mark");
    await user.click(mark ?? document.body);
    const dialog = screen.getByRole("dialog", { name: "加入词典" });
    expect(within(dialog).getByLabelText("曾听成")).toHaveValue("维他命");
    const term = within(dialog).getByLabelText("正确写法");
    expect(term).toHaveValue("Voltip");
    expect(term).toHaveFocus();
    expect(within(dialog).getByTestId("fix-term-from")).toHaveTextContent("来自润色结果");
    // The words stay selected while the focus is in the card.
    expect(
      screen.getByTestId("entry-text").querySelector("[data-fix-highlight]"),
    ).toHaveTextContent("维他命");
    expect(within(dialog).getByText("正在计算影响范围…")).toBeInTheDocument();
    expect(
      await within(dialog).findByText("除本条外，最近 3 条记录中有 2 条含「维他命」。"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText("其中 1 条的最终文本仍为「维他命」，可能是正确用法。"),
    ).toBeInTheDocument();
    expect(within(dialog).getByTestId("fix-status")).toHaveAttribute("data-tone", "warn");
    await user.click(within(dialog).getByRole("button", { name: "查看" }));
    const examples = within(dialog).getByTestId("fix-examples");
    expect(within(examples).getAllByRole("listitem")).toHaveLength(2);
    expect(examples).toHaveTextContent("维他命的安装包发给我");
    await user.click(within(dialog).getByRole("button", { name: "收起" }));
    expect(within(dialog).queryByTestId("fix-examples")).toBeNull();
    expect(within(dialog).getByTestId("fix-note")).toHaveTextContent(
      "识别完成后立即纠正；正确写法还会作为术语发送给云端识别和 AI 润色。",
    );
    await user.click(within(dialog).getByRole("button", { name: "加入词典" }));
    await waitFor(() => {
      expect(backend.peek().dictionary).toEqual([
        expect.objectContaining({
          term: "Voltip",
          heard_as: ["维他命"],
          source: { kind: "history", history_id: SAMPLE.id },
        }),
      ]);
    });
    expect(screen.queryByTestId("fix-card")).toBeNull();
    expect(screen.getByText("已加入词典 · Voltip")).toBeInTheDocument();
    // The mark is the dictionary's now; the legend counts it there.
    await waitFor(() => {
      expect(screen.getAllByTestId("fix-mark")[0]).toHaveAttribute("data-mark", "known");
    });
    expect(screen.getByTestId("fix-legend")).toHaveTextContent("润色时已改正 1 处已加入词典 1 处");
  });

  it("the toast's 撤销 and Ctrl Z take a new entry, a merge and a rule back", async () => {
    const voltip = entry("Voltip", ["沃尔提普"]);
    const { user, backend } = setup({ dictionary: [voltip] });
    await showRaw(user);
    // A merge: the right spelling is an entry already.
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    const dialog = card();
    expect(
      await within(dialog).findByText(
        "「Voltip」已在词典中，将把「维他命」加入它的曾听成写法（现有 1 个）。",
      ),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "添加到「Voltip」" }));
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃尔提普", "维他命"]);
    });
    await user.keyboard("{Control>}z{/Control}");
    await waitFor(() => {
      expect(backend.peek().dictionary[0]?.heard_as).toEqual(["沃尔提普"]);
    });
    expect(screen.queryByText("已加入词典 · Voltip")).toBeNull();
    // A new entry, taken back by the toast's button.
    await user.click(screen.getAllByTestId("fix-mark")[1] ?? document.body);
    await user.click(within(card()).getByRole("button", { name: "加入词典" }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip", "Kubernetes"]);
    });
    await user.click(screen.getByRole("button", { name: /撤销/ }));
    await waitFor(() => {
      expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip"]);
    });
    // Nothing left to undo: Ctrl Z does nothing more.
    await user.keyboard("{Control>}z{/Control}");
    expect(backend.peek().dictionary.map((e) => e.term)).toEqual(["Voltip"]);
  });

  it("selected words get the toolbar; R opens the rule card with the words, a name, the preview and the effect", async () => {
    const { user, backend } = setup();
    await showRaw(user);
    selectWord("非书");
    const toolbar = screen.getByRole("toolbar", { name: "选中文字的操作" });
    expect(within(toolbar).getByRole("button", { name: "加入词典" })).toBeEnabled();
    expect(within(toolbar).getByRole("button", { name: "新建规则" })).toHaveAttribute(
      "aria-keyshortcuts",
      "R",
    );
    await user.keyboard("r");
    const dialog = screen.getByRole("dialog", { name: "新建规则" });
    expect(screen.queryByRole("toolbar")).toBeNull();
    expect(within(dialog).getByLabelText("查找")).toHaveValue("非书");
    const replace = within(dialog).getByLabelText("替换为");
    expect(replace).toHaveFocus();
    expect(within(dialog).getByLabelText("名称")).toHaveValue("非书 →");
    expect(within(dialog).getByRole("switch", { name: "区分大小写" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.type(replace, "飞书");
    expect(within(dialog).getByLabelText("名称")).toHaveValue("非书 → 飞书");
    const preview = await within(dialog).findByTestId("fix-preview");
    await waitFor(() => {
      expect(preview).toHaveTextContent("…比上周低了一半，结果发到非飞书群了，记得把配置同步到…");
    });
    expect(
      await within(dialog).findByText("除本条外，最近 3 条记录中有 1 条会被改动。"),
    ).toBeInTheDocument();
    expect(within(dialog).getByTestId("fix-note")).toHaveTextContent("替换规则在 AI 润色之后执行");
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(backend.peek().rules).toEqual([
        expect.objectContaining({
          name: "非书 → 飞书",
          pattern: "非书",
          replacement: "飞书",
          enabled: true,
        }),
      ]);
    });
    expect(screen.getByText("已新建规则 · 非书 → 飞书")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /撤销/ }));
    await waitFor(() => {
      expect(backend.peek().rules).toEqual([]);
    });
  });

  it("the rule card names a regex the core cannot compile and says when the entry has no match", async () => {
    const { user } = setup();
    await showRaw(user);
    selectWord("非书");
    await user.click(within(screen.getByRole("toolbar")).getByRole("button", { name: "新建规则" }));
    const dialog = card();
    const find = within(dialog).getByLabelText("查找");
    await user.clear(find);
    await user.type(find, "不存在");
    expect(await within(dialog).findByText("本条最终文本中没有「不存在」")).toBeInTheDocument();
    expect(
      await within(dialog).findByText("除本条外，最近 3 条记录都不会被改动。"),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("radio", { name: "正则" }));
    await user.clear(find);
    await user.type(find, "(");
    expect(await within(dialog).findByText(/正则无法编译/)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "保存规则" })).toBeDisabled();
    // An empty pattern or name cannot be saved either.
    await user.clear(find);
    expect(within(dialog).getByRole("button", { name: "保存规则" })).toBeDisabled();
    await user.click(within(dialog).getByRole("switch", { name: "区分大小写" }));
    expect(within(dialog).getByRole("switch", { name: "区分大小写" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("the card's kind switch carries the words over and the name clash is said under the field", async () => {
    const taken: ReplacementRule = {
      id: "00000000-0000-4000-a000-000000000001",
      name: "维他命 → Voltip",
      kind: "literal",
      pattern: "x",
      replacement: "y",
      case_sensitive: true,
      enabled: true,
      created_at_ms: T,
      updated_at_ms: T,
    };
    const { user } = setup({ rules: [taken] });
    await showRaw(user);
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    await user.click(within(card()).getByRole("radio", { name: "替换规则" }));
    const dialog = screen.getByRole("dialog", { name: "新建规则" });
    expect(within(dialog).getByLabelText("查找")).toHaveValue("维他命");
    expect(within(dialog).getByLabelText("替换为")).toHaveValue("Voltip");
    // The generated name is free; a typed one that is taken is not.
    expect(within(dialog).getByLabelText("名称")).toHaveValue("维他命 → Voltip (2)");
    const name = within(dialog).getByLabelText("名称");
    await user.clear(name);
    await user.type(name, "维他命 → Voltip");
    expect(within(dialog).getByText("已有同名规则")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("radio", { name: "词典" }));
    const back = screen.getByRole("dialog", { name: "加入词典" });
    expect(within(back).getByLabelText("曾听成")).toHaveValue("维他命");
    expect(within(back).getByLabelText("正确写法")).toHaveValue("Voltip");
  });

  it("a selection too long or across lines cannot be a dictionary form; Esc drops the selection", async () => {
    const long = "长".repeat(70);
    const { user } = setup({ history: [take(1, `${long}\n第二行`, `${long}\n第二行。`)] });
    await showRaw(user);
    selectWord(long);
    const toolbar = screen.getByRole("toolbar");
    expect(within(toolbar).getByRole("button", { name: "加入词典" })).toBeDisabled();
    expect(screen.getByTestId("fix-toolbar-reason")).toHaveTextContent("词典写法最多 64 个字符");
    // D does nothing then; R still works.
    await user.keyboard("d");
    expect(screen.queryByTestId("fix-card")).toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("toolbar")).toBeNull();
    expect(window.getSelection()?.isCollapsed).toBe(true);
    selectWord(long.slice(0, 3));
    expect(
      within(screen.getByRole("toolbar")).getByRole("button", { name: "加入词典" }),
    ).toBeEnabled();
  });

  it("a selection across lines says so; a selection that is only punctuation gets no toolbar", async () => {
    const { user } = setup({ history: [take(1, "第一行\n第二行，", "第一行\n第二行。")] });
    await showRaw(user);
    selectWord("第一行\n第二行");
    expect(screen.getByTestId("fix-toolbar-reason")).toHaveTextContent("请在一行内选择");
    selectWord("，");
    expect(screen.queryByRole("toolbar")).toBeNull();
  });

  it("in the polished text, words only the polish wrote suggest a rule", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("radio", { name: "润色后" }));
    selectWord("olti");
    await user.keyboard("r");
    const dialog = screen.getByRole("dialog", { name: "新建规则" });
    expect(within(dialog).getByLabelText("查找")).toHaveValue("Voltip");
    expect(within(dialog).getByTestId("fix-polish-only")).toHaveTextContent(
      "这段文字由 AI 润色产生，原文中没有，因此建议使用替换规则。",
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("fix-card")).toBeNull();
    // Words the original has too: the dictionary.
    selectWord("非书");
    await user.keyboard("d");
    const dict = screen.getByRole("dialog", { name: "加入词典" });
    expect(within(dict).getByLabelText("曾听成")).toHaveValue("非书");
    expect(within(dict).getByLabelText("正确写法")).toHaveValue("");
    expect(within(dict).queryByTestId("fix-term-from")).toBeNull();
  });

  it("a selection inside a replacement grows to the whole of it", async () => {
    const { user } = setup();
    await showRaw(user);
    selectWord("他命");
    expect(window.getSelection()?.toString()).toBe("维他命");
    await user.click(within(screen.getByRole("toolbar")).getByRole("button", { name: "加入词典" }));
    expect(within(card()).getByLabelText("正确写法")).toHaveValue("Voltip");
  });

  it("the diff view asks which spelling is right; the original one becomes the right spelling", async () => {
    const { user, backend } = setup();
    await user.click(await screen.findByRole("radio", { name: "对比" }));
    expect(screen.getByTestId("fix-diff-hint")).toHaveTextContent("点按一组改动可加入词典");
    const pairs = screen.getAllByTestId("fix-pair");
    expect(pairs).toHaveLength(2);
    expect(pairs[0]).toHaveAccessibleName("改动：维他命 → Voltip，点按选择正确写法");
    await user.click(pairs[0] ?? document.body);
    const dialog = screen.getByRole("dialog", { name: "哪一种写法正确？" });
    const polished = within(dialog).getByRole("radio", {
      name: "「Voltip」（润色后）· 识别有误，把「维他命」加入词典",
    });
    expect(polished).toBeChecked();
    expect(within(dialog).getByLabelText("曾听成")).toHaveValue("维他命");
    expect(within(dialog).getByLabelText("正确写法")).toHaveValue("Voltip");
    await user.click(
      within(dialog).getByRole("radio", { name: "「维他命」（原文）· 润色改错了，保留原来的写法" }),
    );
    expect(within(dialog).getByLabelText("正确写法")).toHaveValue("维他命");
    expect(within(dialog).getByTestId("fix-term-from")).toHaveTextContent("来自原文");
    expect(within(dialog).getByLabelText("曾听成（可选）")).toHaveValue("");
    expect(within(dialog).getByTestId("fix-note")).toHaveTextContent(
      "正确写法会作为术语发送给 AI 润色，之后润色会保留这个写法。",
    );
    // Nothing to correct, nothing to count.
    expect(within(dialog).queryByTestId("fix-status")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "加入词典" }));
    await waitFor(() => {
      expect(backend.peek().dictionary).toEqual([
        expect.objectContaining({ term: "维他命", heard_as: [] }),
      ]);
    });
  });

  it("the diff view: a deletion alone is no button, Enter on a replacement asks, ✕ closes the cards", async () => {
    const { user } = setup({
      history: [take(1, "嗯，下午用维他命测了酷伯内特斯", "下午用 Voltip 测了酷伯内特斯。")],
      dictionary: [entry("Kubernetes", ["酷伯内特斯"])],
    });
    await user.click(await screen.findByRole("radio", { name: "对比" }));
    const [pair] = screen.getAllByTestId("fix-pair");
    if (pair === undefined) throw new Error("no pair");
    expect(screen.getAllByTestId("fix-pair")).toHaveLength(1);
    expect(screen.getByText("嗯，")).not.toHaveAttribute("role");
    act(() => {
      pair.focus();
    });
    await user.keyboard("{Enter}");
    const which = screen.getByRole("dialog", { name: "哪一种写法正确？" });
    await user.click(within(which).getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    // A dictionary mark's card closes on its ✕ too.
    await user.click(screen.getByRole("radio", { name: "原文" }));
    const known = screen
      .getAllByTestId("fix-mark")
      .find((m) => m.getAttribute("data-mark") === "known");
    if (known === undefined) throw new Error("no dictionary mark");
    await user.click(known);
    const knownCard = screen.getByRole("dialog", { name: "已加入词典" });
    await user.click(within(knownCard).getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("refuses a form another entry owns, and links to the dictionary", async () => {
    const vitamin = entry("维生素", ["维他命"], { enabled: false });
    const { user } = setup({ dictionary: [vitamin] });
    await showRaw(user);
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    const dialog = card();
    expect(
      within(dialog).getByText("「维他命」已是词条「维生素」的曾听成写法。"),
    ).toBeInTheDocument();
    expect(within(dialog).getByTestId("fix-status")).toHaveAttribute("data-tone", "error");
    expect(within(dialog).getByRole("button", { name: "加入词典" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "打开词典" }));
    expect(await screen.findByTestId("page-dictionary")).toBeInTheDocument();
  });

  it("the other refusals and risks say why", async () => {
    const qwen = entry("Qwen", []);
    const voltip = entry("Voltip", ["沃尔提普"]);
    const { user } = setup({ dictionary: [qwen, voltip] });
    await showRaw(user);
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    const dialog = card();
    const heard = within(dialog).getByLabelText("曾听成");
    const term = within(dialog).getByLabelText("正确写法");
    await user.clear(heard);
    await user.type(heard, "qwen");
    expect(
      within(dialog).getByText("「qwen」是另一个词条的正确写法，不能作为曾听成写法。"),
    ).toBeInTheDocument();
    await user.clear(heard);
    await user.type(heard, "沃尔提普");
    expect(
      within(dialog).getByText("「沃尔提普」已是词条「Voltip」的曾听成写法。"),
    ).toBeInTheDocument();
    await user.clear(heard);
    expect(within(dialog).getByText("「Voltip」已是词典中的正确写法。")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "添加到「Voltip」" })).toBeDisabled();
    await user.type(term, "x");
    expect(within(dialog).queryByText("「Voltip」已是词典中的正确写法。")).toBeNull();
    await user.clear(term);
    await user.type(term, "Voltip");
    await user.click(within(dialog).getByRole("button", { name: "查看词条" }));
    expect(await screen.findByTestId("page-dictionary")).toBeInTheDocument();
  });

  it("a short form is a risk worth a warning", async () => {
    const { user } = setup({
      history: [take(1, "我用了维", "我用了 V"), take(2, "与此无关", "与此无关。")],
    });
    await showRaw(user);
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    const dialog = card();
    expect(
      await within(dialog).findByText("写法只有 1 个字，容易误改其他词语。"),
    ).toBeInTheDocument();
    const heard = within(dialog).getByLabelText("曾听成");
    await user.clear(heard);
    await user.type(heard, "ab");
    expect(
      await within(dialog).findByText("写法只有 2 个字母，容易误改其他词语。"),
    ).toBeInTheDocument();
  });

  it("a click outside closes a card nobody typed in; a typed-in card waits for 取消 or Esc", async () => {
    const { user } = setup();
    await showRaw(user);
    const [first] = screen.getAllByTestId("fix-mark");
    await user.click(first ?? document.body);
    expect(card()).toBeInTheDocument();
    await user.click(screen.getByTestId("history-banner-title"));
    expect(screen.queryByTestId("fix-card")).toBeNull();
    await user.click(first ?? document.body);
    await user.type(within(card()).getByLabelText("正确写法"), "x");
    await user.click(screen.getByTestId("history-banner-title"));
    expect(card()).toBeInTheDocument();
    // Another mark does not replace it either.
    await user.click(screen.getAllByTestId("fix-mark")[1] ?? document.body);
    expect(within(card()).getByLabelText("曾听成")).toHaveValue("维他命");
    await user.click(within(card()).getByRole("button", { name: "取消" }));
    expect(screen.queryByTestId("fix-card")).toBeNull();
    expect(first).toHaveFocus();
  });

  it("the keyboard: Enter or Space on a mark, Esc back to it, Tab stays in the card", async () => {
    const { user } = setup();
    await showRaw(user);
    const [first] = screen.getAllByTestId("fix-mark");
    first?.focus();
    await user.keyboard("{Enter}");
    expect(card()).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("fix-card")).toBeNull();
    expect(first).toHaveFocus();
    await user.keyboard(" ");
    const dialog = card();
    // From the last control Tab goes back to the first; Shift Tab the other way.
    const save = within(dialog).getByRole("button", { name: "加入词典" });
    save.focus();
    await user.tab();
    expect(within(dialog).getByRole("radio", { name: "词典" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(save).toHaveFocus();
    // Esc from outside the card closes it too.
    act(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("fix-card")).toBeNull();
  });

  it("a dictionary mark shows its entry and the way to edit it", async () => {
    const voltip = entry("Voltip", ["维他命"]);
    const { user } = setup({ dictionary: [voltip] });
    await showRaw(user);
    const [known] = screen.getAllByTestId("fix-mark");
    expect(known).toHaveAccessibleName("已加入词典：维他命，正确写法为 Voltip");
    await user.hover(known ?? document.body);
    expect(screen.getByTestId("fix-tip")).toHaveTextContent("正确写法为「Voltip」· 点按查看");
    await user.click(known ?? document.body);
    const dialog = screen.getByRole("dialog", { name: "已加入词典" });
    expect(dialog).toHaveTextContent("正确写法Voltip");
    expect(dialog).toHaveTextContent("曾听成维他命");
    expect(within(dialog).getByText(/次$/)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "已加入词典" })).toBeNull();
    await user.click(known ?? document.body);
    await user.click(
      within(screen.getByRole("dialog", { name: "已加入词典" })).getByRole("button", {
        name: "在词典中编辑",
      }),
    );
    expect(await screen.findByTestId("page-dictionary")).toBeInTheDocument();
  });

  it("only takes this computer recognised get the tools (section 16.9 3.8)", async () => {
    const standalone = take(1, RAW, TEXT, { origin: { device: "Pixel 8", kind: "standalone" } });
    const typed = take(2, "手机打的字", "手机打的字", {
      origin: { device: "Pixel 8", kind: "typed" },
    });
    const phoneTake = take(3, RAW, TEXT, { origin: { device: "Pixel 8", kind: "take" } });
    const { user } = setup({ history: [standalone, typed, phoneTake] });
    await showRaw(user);
    expect(screen.getByTestId("fix-not-here")).toHaveTextContent(
      "这条由手机识别。这里的词典只作用于这台电脑上的识别。",
    );
    expect(screen.queryAllByTestId("fix-mark")).toEqual([]);
    expect(screen.queryByRole("button", { name: "加入词典" })).toBeNull();
    const rows = within(screen.getByRole("list", { name: "听写记录" })).getAllByRole("button", {
      pressed: false,
    });
    await user.click(rows[0] ?? document.body);
    expect(screen.queryByTestId("fix-not-here")).toBeNull();
    expect(screen.queryByRole("button", { name: "加入词典" })).toBeNull();
    await user.click(rows[1] ?? document.body);
    expect(screen.getAllByTestId("fix-mark")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "加入词典" })).toBeInTheDocument();
  });

  it("the detail's own 加入词典 takes the tracked selection, next to the button", async () => {
    const { user } = setup();
    await showRaw(user);
    selectWord("非书");
    const own = screen
      .getAllByRole("button", { name: "加入词典" })
      .find((b) => !b.hasAttribute("aria-keyshortcuts"));
    await user.click(own ?? document.body);
    expect(within(card()).getByLabelText("曾听成")).toHaveValue("非书");
    expect(screen.queryByRole("toolbar")).toBeNull();
  });

  it("a double click selects the segmenter's word", async () => {
    const { user } = setup({
      history: [take(1, "please attach the report", "Please attach the report.")],
    });
    await showRaw(user);
    const root = screen.getByTestId("entry-text").querySelector("[data-fix-text]");
    const leaf = [...(root?.querySelectorAll("[data-at]") ?? [])].find((el) =>
      (el.textContent ?? "").includes("attach"),
    );
    const node = leaf?.firstChild;
    if (root == null || leaf === undefined || node == null) throw new Error("no text");
    const at = (leaf.textContent ?? "").indexOf("attach") + 2;
    const range = document.createRange();
    range.setStart(node, at);
    range.setEnd(node, at);
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
    fireEvent.doubleClick(root);
    expect(window.getSelection()?.toString()).toBe("attach");
    expect(screen.getByRole("toolbar")).toBeInTheDocument();
  });

  it("the toolbar goes when the page scrolls or the view changes", async () => {
    const { user } = setup();
    await showRaw(user);
    selectWord("非书");
    expect(screen.getByRole("toolbar")).toBeInTheDocument();
    fireEvent.scroll(window);
    expect(screen.queryByRole("toolbar")).toBeNull();
    selectWord("非书");
    window.getSelection()?.removeAllRanges();
    fireEvent(document, new Event("selectionchange"));
    expect(screen.queryByRole("toolbar")).toBeNull();
    selectWord("非书");
    await user.click(screen.getByRole("radio", { name: "润色后" }));
    expect(screen.queryByRole("toolbar")).toBeNull();
  });

  it("a backtest the core refuses is said in the card", async () => {
    const { user, backend } = setup();
    vi.spyOn(backend, "vocabularyBacktest").mockRejectedValue(new Error("history: 数据库不可用"));
    await showRaw(user);
    await user.click(screen.getAllByTestId("fix-mark")[0] ?? document.body);
    expect(await within(card()).findByText("数据库不可用")).toBeInTheDocument();
  });
});
