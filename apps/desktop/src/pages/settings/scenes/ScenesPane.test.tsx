import { type AppRef, type HistoryEntry, MAX_SCENES, type Scene } from "@voltip/shared";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../../test/render";

const NOW = 1_758_700_000_000;
const CHAT_ID = "00000000-0000-4000-a000-000000000001";
const GITHUB_ID = "00000000-0000-4000-a000-000000000002";

function scene(id: string, name: string, extra: Partial<Scene> = {}): Scene {
  return {
    id,
    name,
    enabled: true,
    match: { apps: ["app"], title_contains: [] },
    overrides: {},
    created_at_ms: NOW,
    updated_at_ms: NOW,
    ...extra,
  };
}

/** Chat apps: refine off, punctuation only, an instruction. */
const CHAT = scene(CHAT_ID, "聊天", {
  match: { apps: ["slack", "wechat"], title_contains: [] },
  overrides: { refine_enabled: false, refine_style: "punctuation", prompt: "口语化" },
});
/** GitHub in the browser, switched off: streaming, English, Traditional. */
const GITHUB = scene(GITHUB_ID, "GitHub", {
  enabled: false,
  match: { apps: ["chrome"], title_contains: ["GitHub", "Pull request"] },
  overrides: { output_mode: "streaming_final", language: "en", chinese_script: "traditional" },
});

function take(n: number, app?: AppRef): HistoryEntry {
  return {
    id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
    at_ms: NOW - n * 60_000,
    raw_text: "好的",
    text: "好的。",
    refined: false,
    asr_model: "Qwen/Qwen3-ASR-1.7B",
    duration_ms: 1200,
    asr_ms: 300,
    outcome: { kind: "inserted", via: "paste" },
    starred: false,
    mode: "whole_take",
    kind: "dictation",
    ...(app === undefined ? {} : { app }),
  };
}

async function scenesPane() {
  const dialog = await screen.findByRole("dialog", { name: "设置" });
  return { dialog, pane: within(dialog).getByTestId("scenes-pane") };
}

function cards(pane: HTMLElement): HTMLElement[] {
  return within(within(pane).getByRole("list", { name: "场景列表" })).getAllByRole("article");
}

function card(pane: HTMLElement, index: number): HTMLElement {
  const found = cards(pane)[index];
  if (found === undefined) throw new Error(`no scene card at ${index}`);
  return found;
}

function names(pane: HTMLElement): (string | null)[] {
  return cards(pane).map((c) => c.getAttribute("aria-label"));
}

describe("Settings · 场景 (docs/dictation.md section 18)", () => {
  it("regression: the 场景 group lists the scenes in matching order with what they match and override; the switch, the order buttons and delete run the scene commands", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/scene", mock: { scenes: [CHAT, GITHUB] } });
    const { dialog, pane } = await scenesPane();
    expect(within(dialog).getByRole("tab", { name: /场景/, selected: true })).toBeInTheDocument();
    expect(within(pane).getByRole("heading", { name: "场景", level: 2 })).toBeInTheDocument();
    expect(within(dialog).getByTestId("settings-readouts")).toHaveTextContent(
      "场景 2 个 · 启用 1 · 上下文 应用名称 开 · 窗口标题 关",
    );
    expect(names(pane)).toEqual(["聊天", "GitHub"]);
    const chat = card(pane, 0);
    const github = card(pane, 1);
    expect(within(chat).getByText("slack")).toBeInTheDocument();
    expect(within(chat).getByText("wechat")).toBeInTheDocument();
    expect(within(chat).getByTestId("scene-match")).toHaveTextContent("任何窗口");
    expect(within(chat).getByTestId("scene-summary")).toHaveTextContent(
      "AI 润色 关 · 润色：只加标点 · 有补充要求",
    );
    expect(chat).toHaveAttribute("data-enabled", "true");
    expect(within(github).getByTestId("scene-match")).toHaveTextContent(
      "chrome窗口标题含 GitHub · Pull request",
    );
    expect(within(github).getByTestId("scene-summary")).toHaveTextContent(
      "输出：边说边识别 · 语言：English · en · 字形：繁体",
    );
    expect(github).toHaveAttribute("data-enabled", "false");
    expect(within(chat).getByRole("button", { name: "上移 聊天" })).toBeDisabled();
    expect(within(github).getByRole("button", { name: "下移 GitHub" })).toBeDisabled();

    const invoke = vi.spyOn(backend, "invoke");
    // The switch sends the whole draft with only `enabled` changed.
    await user.click(within(github).getByRole("switch", { name: "启用 GitHub" }));
    expect(invoke).toHaveBeenLastCalledWith("scenes_update", {
      id: GITHUB_ID,
      scene: {
        name: "GitHub",
        enabled: true,
        match: GITHUB.match,
        overrides: GITHUB.overrides,
      },
    });
    await waitFor(() => {
      expect(cards(pane)[1]).toHaveAttribute("data-enabled", "true");
    });
    expect(within(dialog).getByTestId("settings-readouts")).toHaveTextContent("场景 2 个 · 启用 2");

    // ↓ on the first card swaps the two.
    await user.click(within(chat).getByRole("button", { name: "下移 聊天" }));
    expect(invoke).toHaveBeenLastCalledWith("scenes_reorder", { ids: [GITHUB_ID, CHAT_ID] });
    await waitFor(() => {
      expect(names(pane)).toEqual(["GitHub", "聊天"]);
    });
    await user.click(within(card(pane, 1)).getByRole("button", { name: "上移 聊天" }));
    expect(invoke).toHaveBeenLastCalledWith("scenes_reorder", { ids: [CHAT_ID, GITHUB_ID] });
    await waitFor(() => {
      expect(names(pane)).toEqual(["聊天", "GitHub"]);
    });

    // Delete asks first; cancelling keeps the scene.
    await user.click(within(card(pane, 0)).getByRole("button", { name: "删除 聊天" }));
    let confirm = await screen.findByRole("dialog", { name: "删除场景「聊天」？" });
    expect(confirm).toHaveTextContent("之后在这些应用里的听写按全局设置处理");
    await user.click(within(confirm).getByRole("button", { name: "取消" }));
    expect(names(pane)).toEqual(["聊天", "GitHub"]);
    await user.click(within(card(pane, 0)).getByRole("button", { name: "删除 聊天" }));
    confirm = await screen.findByRole("dialog", { name: "删除场景「聊天」？" });
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    expect(invoke).toHaveBeenLastCalledWith("scenes_remove", { id: CHAT_ID });
    await waitFor(() => {
      expect(names(pane)).toEqual(["GitHub"]);
    });
    // The settings dialog stays open under the confirm.
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
  });

  it("regression: the context switches start at app name on and window title off and write settings_set_context_sharing; an empty list shows its empty state", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/scene" });
    const { dialog, pane } = await scenesPane();
    const context = within(pane).getByTestId("context-sharing");
    expect(context).toHaveTextContent("发送给 AI 润色的上下文");
    const appName = within(context).getByRole("switch", { name: "应用名称" });
    const title = within(context).getByRole("switch", { name: "窗口标题" });
    expect(appName).toHaveAttribute("aria-checked", "true");
    expect(title).toHaveAttribute("aria-checked", "false");

    const invoke = vi.spyOn(backend, "invoke");
    await user.click(title);
    expect(invoke).toHaveBeenLastCalledWith("settings_set_context_sharing", {
      appName: true,
      windowTitle: true,
    });
    await waitFor(() => {
      expect(title).toHaveAttribute("aria-checked", "true");
    });
    await user.click(appName);
    expect(invoke).toHaveBeenLastCalledWith("settings_set_context_sharing", {
      appName: false,
      windowTitle: true,
    });
    await waitFor(() => {
      expect(appName).toHaveAttribute("aria-checked", "false");
    });
    expect(within(dialog).getByTestId("settings-readouts")).toHaveTextContent(
      "场景 0 个 · 启用 0 · 上下文 应用名称 关 · 窗口标题 开",
    );

    expect(within(pane).queryByRole("list", { name: "场景列表" })).toBeNull();
    expect(within(pane).getByText("暂无场景")).toBeInTheDocument();
    expect(within(pane).getByText("上限 50 个 · 从上到下匹配")).toBeInTheDocument();
  });

  it("regression: 新建场景 builds a scene from a typed id and a recent app, title keywords and overrides that start at 跟随全局, and saves it through scenes_add", async () => {
    const user = userEvent.setup();
    const history = [
      take(1, { id: "code", name: "Code" }),
      take(2),
      take(3, { id: "slack", name: "Slack" }),
      take(4, { id: "code", name: "Code" }),
    ];
    const { backend } = renderApp({ path: "/settings/scene", mock: { history } });
    const { pane } = await scenesPane();
    const invoke = vi.spyOn(backend, "invoke");
    await user.click(within(pane).getByRole("button", { name: "新建场景" }));
    const editor = await screen.findByRole("dialog", { name: "新建场景" });
    const name = within(editor).getByRole("textbox", { name: "名称" });
    expect(name).toHaveFocus();

    // Nothing filled in: saving shows what is missing and sends nothing.
    expect(within(editor).queryByText("请填写名称")).toBeNull();
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    expect(within(editor).getByText("请填写名称")).toBeInTheDocument();
    expect(within(editor).getByText("至少添加一个应用")).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("scenes_add", expect.anything());

    await user.type(name, "编程");
    expect(within(editor).queryByText("请填写名称")).toBeNull();
    // A typed id is normalised (case, `.exe`); Enter adds it.
    const apps = within(editor).getByRole("textbox", { name: "应用" });
    await user.type(apps, "Code.EXE{Enter}");
    expect(apps).toHaveValue("");
    const appChips = () =>
      within(within(editor).getByRole("list", { name: "应用" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent);
    expect(appChips()).toEqual(["code"]);
    expect(within(editor).queryByText("至少添加一个应用")).toBeNull();
    // 最近的应用 comes from the history, newest first, one per app; a chip toggles its app.
    const recent = within(editor).getByRole("group", { name: "最近的应用" });
    expect(
      within(recent)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Code", "Slack"]);
    expect(within(recent).getByRole("button", { name: "Code" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.click(within(recent).getByRole("button", { name: "Slack" }));
    expect(appChips()).toEqual(["code", "slack"]);
    await user.click(within(recent).getByRole("button", { name: "Slack" }));
    expect(appChips()).toEqual(["code"]);
    await user.click(within(recent).getByRole("button", { name: "Slack" }));
    expect(within(recent).getByRole("button", { name: "Slack" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // The 添加 button does what Enter does; a duplicate is ignored.
    await user.type(apps, " slack ");
    await user.click(
      within(within(editor).getByTestId("scene-editor-apps")).getByRole("button", {
        name: "添加",
      }),
    );
    expect(appChips()).toEqual(["code", "slack"]);

    // Title keywords: trimmed, one per spelling ignoring case, removable.
    const keywords = within(editor).getByRole("textbox", { name: "窗口标题关键词" });
    await user.type(keywords, " GitHub {Enter}github{Enter}PR{Enter}");
    const keywordList = within(editor).getByRole("list", { name: "窗口标题关键词" });
    expect(
      within(keywordList)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["GitHub", "PR"]);
    await user.click(within(keywordList).getByRole("button", { name: "移除 PR" }));
    expect(
      within(keywordList)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["GitHub"]);

    // Every override starts at 跟随全局.
    const overrides = within(editor).getByTestId("scene-editor-overrides");
    for (const label of ["AI 润色", "润色风格", "输出方式", "语言", "中文字形"]) {
      expect(within(overrides).getByRole("combobox", { name: label })).toHaveDisplayValue(
        "跟随全局",
      );
    }
    await user.selectOptions(within(overrides).getByRole("combobox", { name: "AI 润色" }), "关");
    // A streaming mode without the streaming model runs whole takes, and the editor says so.
    expect(within(editor).queryByTestId("scene-streaming-note")).toBeNull();
    await user.selectOptions(
      within(overrides).getByRole("combobox", { name: "输出方式" }),
      "边说边识别",
    );
    expect(within(editor).getByTestId("scene-streaming-note")).toHaveTextContent(
      "实时识别模型未下载时，本场景按整段输出运行",
    );
    const prompt = within(editor).getByRole("textbox", { name: "给 AI 的补充要求" });
    await user.type(prompt, "保留英文标识符");
    expect(within(editor).getByTestId("scene-prompt-count")).toHaveTextContent("7 / 500");

    // Ctrl S saves like the button.
    await user.keyboard("{Control>}s{/Control}");
    expect(invoke).toHaveBeenLastCalledWith("scenes_add", {
      scene: {
        name: "编程",
        enabled: true,
        match: { apps: ["code", "slack"], title_contains: ["GitHub"] },
        overrides: {
          refine_enabled: false,
          output_mode: "streaming_final",
          prompt: "保留英文标识符",
        },
      },
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "新建场景" })).toBeNull();
    });
    expect(await screen.findByText("已保存场景 · 编程")).toBeInTheDocument();
    const created = within(pane).getByRole("article", { name: "编程" });
    expect(within(created).getByTestId("scene-match")).toHaveTextContent(
      "codeslack窗口标题含 GitHub",
    );
    expect(within(created).getByTestId("scene-summary")).toHaveTextContent(
      "AI 润色 关 · 输出：边说边识别 · 有补充要求",
    );
  });

  it("regression: the editor refuses a duplicate name and a long prompt before sending, shows the core's refusal and stays open; editing sends scenes_update with the id and the untouched overrides; Esc closes only the editor", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/scene", mock: { scenes: [CHAT, GITHUB] } });
    const { pane } = await scenesPane();
    const invoke = vi.spyOn(backend, "invoke");
    await user.click(within(pane).getByRole("button", { name: "新建场景" }));
    let editor = await screen.findByRole("dialog", { name: "新建场景" });
    // No history app yet: the recent list says where they come from.
    expect(editor).toHaveTextContent("历史记录中暂无应用；完成一次听写后会显示在这里。");
    const name = within(editor).getByRole("textbox", { name: "名称" });
    // A clash is shown as soon as it is typed (ASCII case ignored, like the core).
    await user.type(name, "github");
    expect(within(editor).getByText("已有名为「GitHub」的场景")).toBeInTheDocument();
    await user.type(within(editor).getByRole("textbox", { name: "应用" }), "chrome{Enter}");
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    expect(invoke).not.toHaveBeenCalled();

    // Over 500 characters: the counter and the note turn red and nothing is sent.
    await user.clear(name);
    await user.type(name, "代码");
    const prompt = within(editor).getByRole("textbox", { name: "给 AI 的补充要求" });
    await user.click(prompt);
    await user.paste("长".repeat(501));
    expect(within(editor).getByTestId("scene-prompt-count")).toHaveTextContent("501 / 500");
    expect(within(editor).getByTestId("scene-prompt-count")).toHaveClass("text-danger");
    expect(within(editor).getByText("补充要求最多 500 个字符")).toBeInTheDocument();
    expect(prompt).toHaveAttribute("aria-invalid", "true");
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    expect(invoke).not.toHaveBeenCalled();
    await user.clear(prompt);

    // What only the core checks (here the 32-character name) comes back as its refusal.
    await user.clear(name);
    await user.type(name, "名".repeat(33));
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    expect(await within(editor).findByRole("alert")).toHaveTextContent(
      "场景名称最多 32 个字符（当前 33）",
    );
    expect(screen.getByRole("dialog", { name: "新建场景" })).toBeInTheDocument();
    // Editing again clears the refusal.
    await user.type(name, "{Backspace}");
    expect(within(editor).queryByRole("alert")).toBeNull();

    // Esc closes the editor, not the settings dialog.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "新建场景" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "设置" })).toBeInTheDocument();
    expect(names(pane)).toEqual(["聊天", "GitHub"]);

    // Editing opens with the scene's values.
    await user.click(within(card(pane, 1)).getByRole("button", { name: "编辑 GitHub" }));
    editor = await screen.findByRole("dialog", { name: "编辑场景" });
    expect(within(editor).getByRole("textbox", { name: "名称" })).toHaveValue("GitHub");
    expect(
      within(within(editor).getByRole("list", { name: "窗口标题关键词" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["GitHub", "Pull request"]);
    const overrides = within(editor).getByTestId("scene-editor-overrides");
    const select = (label: string) => within(overrides).getByRole("combobox", { name: label });
    expect(select("AI 润色")).toHaveDisplayValue("跟随全局");
    expect(select("输出方式")).toHaveDisplayValue("边说边识别");
    expect(select("语言")).toHaveDisplayValue("English · en");
    expect(select("中文字形")).toHaveDisplayValue("繁体");
    await user.selectOptions(select("AI 润色"), "开");
    await user.selectOptions(select("语言"), "跟随全局");
    await user.click(
      within(within(editor).getByRole("list", { name: "应用" })).getByRole("button", {
        name: "移除 chrome",
      }),
    );
    await user.type(within(editor).getByRole("textbox", { name: "应用" }), "firefox{Enter}");
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    expect(invoke).toHaveBeenLastCalledWith("scenes_update", {
      id: GITHUB_ID,
      scene: {
        name: "GitHub",
        enabled: false,
        match: { apps: ["firefox"], title_contains: ["GitHub", "Pull request"] },
        overrides: {
          refine_enabled: true,
          output_mode: "streaming_final",
          chinese_script: "traditional",
        },
      },
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "编辑场景" })).toBeNull();
    });
    const github = within(pane).getByRole("article", { name: "GitHub" });
    expect(within(github).getByTestId("scene-summary")).toHaveTextContent(
      "AI 润色 开 · 输出：边说边识别 · 字形：繁体",
    );
    expect(within(github).getByText("firefox")).toBeInTheDocument();
  });

  it("regression: 新建场景 is disabled at the 50-scene cap, and a refused change becomes a toast", async () => {
    const user = userEvent.setup();
    const many = Array.from({ length: MAX_SCENES }, (_, i) =>
      scene(`00000000-0000-4000-a000-${String(i + 1).padStart(12, "0")}`, `s${i}`),
    );
    const { backend } = renderApp({ path: "/settings/scene", mock: { scenes: many } });
    const { pane } = await scenesPane();
    expect(within(pane).getByRole("button", { name: "新建场景" })).toBeDisabled();
    expect(cards(pane)).toHaveLength(MAX_SCENES);
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("scenes: 暂时不能保存"));
    await user.click(within(card(pane, 0)).getByRole("switch", { name: "启用 s0" }));
    expect(await screen.findByText("出错了 · 暂时不能保存")).toBeInTheDocument();
  });
});
