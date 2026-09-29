import { MAX_PRESET_PROMPT_CHARS } from "@voltip/shared";
import {
  MOCK_BUILTIN_PRESET_TEXTS,
  MOCK_ENGINE_BUILTIN,
  MOCK_PRESET_SAMPLES,
} from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../../test/render";
import {
  MANAGE_PRESETS,
  copyDraft,
  presetMenuSections,
  presetProblems,
  sampleProblem,
} from "../../../features/presets/presets";

const WEEKLY = {
  id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
  name: "Weekly",
  prompt: "整理成周报：按项目分组。",
  created_at_ms: 1,
  updated_at_ms: 1,
};

async function section() {
  return screen.findByTestId("presets-section");
}

describe("AI 模型 · 预设 (docs/dictation.md section 21)", () => {
  it("lists the built-in presets with the current one selected; choosing a card saves refine_preset", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/ai" });
    const presets = await section();
    const builtin = within(presets).getByRole("listbox", { name: "内置预设" });
    const cards = within(builtin).getAllByRole("option");
    expect(cards.map((c) => c.querySelector(".font-semibold")?.textContent)).toEqual([
      "校对",
      "提示词优化",
      "意图整理",
      "口语聊天",
      "中英互译",
      "要点纪要",
      "只加标点",
      "书面语",
    ]);
    expect(cards[0]).toHaveAttribute("aria-selected", "true");
    // Each card is named by its preset (the description and the button follow).
    expect(within(builtin).getByRole("option", { name: "提示词优化" })).toBe(cards[1]);
    expect(within(cards[0] as HTMLElement).getByText("使用中")).toBeInTheDocument();
    expect(within(presets).getByTestId("presets-current")).toHaveTextContent("当前：校对");
    expect(within(presets).getByText("把口述的需求改写成清晰的 AI 提示词。")).toBeInTheDocument();
    expect(within(presets).getByTestId("presets-empty")).toBeInTheDocument();
    await user.click(cards[1] as HTMLElement);
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_preset).toBe("prompt");
    });
    // The rest of the engine settings is sent unchanged.
    expect(backend.peek().settings.engines.refine_enabled).toBe(true);
    expect(within(presets).getByTestId("presets-current")).toHaveTextContent("当前：提示词优化");
  });

  it("复制为自定义 opens the editor on the built-in text; the saved copy can be used, edited, and deleted after a confirmation", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/ai" });
    const presets = await section();
    const builtin = within(presets).getByRole("listbox", { name: "内置预设" });
    const promptCard = within(builtin).getAllByRole("option")[1] as HTMLElement;
    await user.click(within(promptCard).getByRole("button", { name: "复制为自定义" }));
    const dialog = await screen.findByRole("dialog", { name: "新建预设" });
    const body = MOCK_BUILTIN_PRESET_TEXTS.find((p) => p.id === "prompt")?.prompt ?? "";
    expect(within(dialog).getByRole("textbox", { name: "名称" })).toHaveValue("提示词优化（副本）");
    expect(within(dialog).getByRole("textbox", { name: "提示词" })).toHaveValue(body);
    // Copying does not switch the preset in use.
    expect(backend.peek().settings.engines.refine_preset).toBe("proofread");
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    expect(await screen.findByText("已保存预设 · 提示词优化（副本）")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "新建预设" })).toBeNull();
    const [copy] = backend.peek().presets;
    expect(copy).toMatchObject({ name: "提示词优化（副本）", prompt: body });
    const custom = within(presets).getByRole("listbox", { name: "自定义预设" });
    await user.click(within(custom).getByRole("option", { name: "提示词优化（副本）" }));
    await waitFor(() => {
      expect(backend.peek().settings.engines.refine_preset).toBe(copy?.id);
    });

    await user.click(within(custom).getByRole("button", { name: "编辑 提示词优化（副本）" }));
    const editor = await screen.findByRole("dialog", { name: "编辑预设" });
    const name = within(editor).getByRole("textbox", { name: "名称" });
    await user.clear(name);
    await user.type(name, "需求整理");
    await user.click(within(editor).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().presets.map((p) => p.name)).toEqual(["需求整理"]);
    });
    expect(within(presets).getByTestId("presets-current")).toHaveTextContent("当前：需求整理");

    await user.click(within(presets).getByRole("button", { name: "删除 需求整理" }));
    const confirm = await screen.findByRole("dialog", { name: "删除预设「需求整理」？" });
    expect(confirm).toHaveTextContent("正在使用这个预设的设置和场景将改用「校对」。");
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().presets).toEqual([]);
    });
    // The settings still name the deleted preset: takes use 校对, and the section says so.
    expect(within(presets).getByTestId("presets-missing")).toHaveTextContent(
      "已删除的预设（按校对处理）",
    );
    expect(within(presets).getByTestId("presets-current")).toHaveTextContent(
      "当前：已删除的预设（按校对处理）",
    );
  });

  it("the editor checks the name and the prompt like the core, counts the prompt, and 试运行 shows the clean-up's answer", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/ai", mock: { presets: [WEEKLY] } });
    const presets = await section();
    await user.click(within(presets).getByRole("button", { name: "新建预设" }));
    const dialog = await screen.findByRole("dialog", { name: "新建预设" });
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    expect(within(dialog).getByText("请填写名称")).toBeInTheDocument();
    expect(within(dialog).getByText("请填写提示词")).toBeInTheDocument();
    // Names are unique ignoring ASCII case, as the core compares them.
    await user.type(within(dialog).getByRole("textbox", { name: "名称" }), "weekly");
    expect(within(dialog).getByText("已有名为「Weekly」的预设")).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox", { name: "提示词" }), "  整理成会议纪要  ");
    expect(within(dialog).getByTestId("preset-prompt-count")).toHaveTextContent(
      `7 / ${MAX_PRESET_PROMPT_CHARS}`,
    );
    expect(backend.peek().presets).toHaveLength(1);

    // 试运行 runs the prompt being edited on the sample, which starts as the 校对 example.
    const trial = within(dialog).getByTestId("preset-trial");
    const sample = within(trial).getByRole("textbox", { name: "示例文字" });
    expect(sample).toHaveValue(MOCK_PRESET_SAMPLES.proofread?.input);
    await user.click(within(trial).getByRole("button", { name: "运行" }));
    const result = await within(trial).findByTestId("preset-trial-result");
    expect(result).toHaveTextContent(MOCK_PRESET_SAMPLES.proofread?.output ?? "");
    expect(result).toHaveTextContent(`结果 · ${MOCK_ENGINE_BUILTIN.refine_model} · 300 ms`);
    // Editing the sample drops the old answer; an empty sample cannot run.
    await user.clear(sample);
    expect(within(trial).queryByTestId("preset-trial-result")).toBeNull();
    expect(within(trial).getByRole("button", { name: "运行" })).toBeDisabled();
    await user.type(sample, "嗯今天下午开会");
    await user.click(within(trial).getByRole("button", { name: "运行" }));
    expect(await within(trial).findByTestId("preset-trial-result")).toHaveTextContent(
      "今天下午开会。",
    );
    // Nothing was saved or recorded by the trial runs.
    expect(backend.peek().presets).toHaveLength(1);
    expect(backend.peek().history.every((h) => h.text !== "今天下午开会。")).toBe(true);

    await user.clear(within(dialog).getByRole("textbox", { name: "名称" }));
    await user.type(within(dialog).getByRole("textbox", { name: "名称" }), "会议纪要");
    await user.click(within(dialog).getByRole("button", { name: /保存/ }));
    await waitFor(() => {
      expect(backend.peek().presets.map((p) => [p.name, p.prompt])).toEqual([
        ["Weekly", WEEKLY.prompt],
        ["会议纪要", "整理成会议纪要"],
      ]);
    });
  });

  it("without an AI polish service the trial run is unavailable and says why", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/ai", mock: { builtIn: {} } });
    const presets = await section();
    await user.click(within(presets).getByRole("button", { name: "新建预设" }));
    const trial = within(await screen.findByRole("dialog", { name: "新建预设" })).getByTestId(
      "preset-trial",
    );
    expect(trial).toHaveTextContent("尚未配置 AI 润色服务，无法试运行。");
    expect(within(trial).getByRole("button", { name: "运行" })).toBeDisabled();
  });

  it("the preset helpers are pure and follow the core's rules", () => {
    const sections = presetMenuSections("prompt", [WEEKLY]);
    expect(sections.map((s) => s.label)).toEqual(["内置预设", "自定义预设", undefined]);
    expect(
      sections[0]?.items.filter((i) => i.kind === "radio" && i.checked).map((i) => i.id),
    ).toEqual(["prompt"]);
    expect(sections[1]?.items).toEqual([
      { kind: "radio", id: WEEKLY.id, label: "Weekly", checked: false, userText: true },
    ]);
    expect(sections[2]?.items[0]?.id).toBe(MANAGE_PRESETS);
    // No custom presets: no empty custom group.
    expect(presetMenuSections("proofread", []).map((s) => s.label)).toEqual([
      "内置预设",
      undefined,
    ]);
    expect(presetProblems({ name: " ", prompt: "" }, [])).toEqual({
      name: { text: "请填写名称", missing: true },
      prompt: { text: "请填写提示词", missing: true },
    });
    expect(presetProblems({ name: "字".repeat(25), prompt: "x".repeat(4001) }, [])).toEqual({
      name: { text: "名称最多 24 个字符", missing: false },
      prompt: { text: "提示词最多 4000 个字符", missing: false },
    });
    expect(presetProblems({ name: " WEEKLY ", prompt: "x" }, [WEEKLY]).name?.text).toBe(
      "已有名为「Weekly」的预设",
    );
    expect(presetProblems({ name: "周报", prompt: "x" }, [WEEKLY])).toEqual({});
    expect(sampleProblem("  ")).toBe("请输入示例文字");
    expect(sampleProblem("字".repeat(2001))).toBe("示例文字最多 2000 个字符");
    expect(sampleProblem("你好")).toBeUndefined();
    expect(copyDraft("notes", "正文")).toEqual({ name: "要点纪要（副本）", prompt: "正文" });
  });
});
