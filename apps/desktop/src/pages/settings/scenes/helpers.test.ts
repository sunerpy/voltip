import {
  MAX_SCENE_APPS,
  MAX_TITLE_KEYWORDS,
  type Scene,
  createTranslator,
  sceneLabel,
  zhT,
} from "@voltip/shared";
import {
  addApp,
  addKeyword,
  editorDraftFrom,
  editorProblems,
  hasProblems,
  languageChoices,
  languageName,
  outputModeChoices,
  overrideSummary,
  promptChars,
  refineChoices,
  sceneDraftOf,
  presetChoices,
  scriptChoices,
  withEnabled,
} from "./helpers";

const en = createTranslator("en").t;

const SCENE: Scene = {
  id: "00000000-0000-4000-a000-000000000001",
  name: "Docs",
  enabled: true,
  match: { apps: ["winword"], title_contains: ["Report"] },
  overrides: {
    refine_enabled: true,
    refine_preset: "formal",
    output_mode: "whole_take",
    language: "auto",
    chinese_script: "as_is",
    prompt: "书面语",
  },
  created_at_ms: 1,
  updated_at_ms: 2,
};

describe("scene editor helpers (docs/dictation.md section 18)", () => {
  it("round-trips a scene through the form and back to the wire draft", () => {
    const form = editorDraftFrom(SCENE);
    expect(form).toEqual({
      name: "Docs",
      enabled: true,
      apps: ["winword"],
      keywords: ["Report"],
      refine: "on",
      preset: "formal",
      outputMode: "whole_take",
      language: "auto",
      script: "as_is",
      prompt: "书面语",
    });
    expect(sceneDraftOf(form)).toEqual({
      name: "Docs",
      enabled: true,
      match: SCENE.match,
      overrides: SCENE.overrides,
    });
    // Unset overrides stay absent; a blank language or prompt is unset; the name is trimmed.
    expect(
      sceneDraftOf({ ...editorDraftFrom(), name: " New ", language: " ", prompt: " \n" }),
    ).toEqual({
      name: "New",
      enabled: true,
      match: { apps: [], title_contains: [] },
      overrides: {},
    });
    expect(editorDraftFrom({ ...SCENE, overrides: { refine_enabled: false } }).refine).toBe("off");
    expect(withEnabled(SCENE, false)).toEqual({
      name: "Docs",
      enabled: false,
      match: SCENE.match,
      overrides: SCENE.overrides,
    });
  });

  it("adds normalised app ids and keywords once", () => {
    expect(addApp([], " Slack.EXE ")).toEqual(["slack"]);
    expect(addApp(["slack"], "SLACK")).toEqual(["slack"]);
    expect(addApp(["slack"], ".exe")).toEqual(["slack"]);
    expect(addKeyword([], " GitHub ")).toEqual(["GitHub"]);
    expect(addKeyword(["GitHub"], "github")).toEqual(["GitHub"]);
    expect(addKeyword(["GitHub"], "  ")).toEqual(["GitHub"]);
    expect(promptChars(" 第一行\r\n第二行 ")).toBe(7);
  });

  it("reports the problems the core would, per field, marking the missing ones", () => {
    const ok = { ...editorDraftFrom(), name: "a", apps: ["x"] };
    expect(hasProblems(editorProblems(ok, []))).toBe(false);
    const empty = editorProblems(editorDraftFrom(), []);
    expect(empty.name).toEqual({ text: "请填写名称", missing: true });
    expect(empty.apps).toEqual({ text: "至少添加一个应用", missing: true });
    const many = editorProblems(
      {
        ...ok,
        apps: Array.from({ length: MAX_SCENE_APPS + 1 }, (_, i) => `a${i}`),
        keywords: Array.from({ length: MAX_TITLE_KEYWORDS + 1 }, (_, i) => `k${i}`),
        prompt: "x".repeat(501),
      },
      [],
      en,
    );
    expect(many).toEqual({
      apps: { text: "At most 20 apps", missing: false },
      keywords: { text: "At most 10 keywords", missing: false },
      prompt: { text: "Extra instructions can be at most 500 characters", missing: false },
    });
    expect(editorProblems({ ...ok, name: "DOCS" }, [SCENE]).name).toEqual({
      text: "已有名为「Docs」的场景",
      missing: false,
    });
  });

  it("offers 跟随全局 first in every select and names languages like the engines dialog", () => {
    for (const choices of [
      refineChoices(),
      presetChoices([], ""),
      outputModeChoices(),
      scriptChoices(),
    ])
      expect(choices[0]).toEqual({ value: "", label: "跟随全局" });
    expect(presetChoices([], "", en).map((c) => c.label)).toEqual([
      "Follow global",
      "Proofread",
      "Prompt optimizer",
      "Clarify intent",
      "Casual chat",
      "Chinese ⇄ English",
      "Key points",
      "Punctuation only",
      "Formal",
    ]);
    // Custom presets follow the built-in ones; one that was deleted stays selectable, named so.
    const weekly = {
      id: "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e",
      name: "周报",
      prompt: "整理成周报",
      created_at_ms: 1,
      updated_at_ms: 1,
    };
    expect(presetChoices([weekly], weekly.id).slice(-1)).toEqual([
      { value: weekly.id, label: "周报" },
    ]);
    const gone = "11111111-1111-4111-8111-111111111111";
    expect(presetChoices([weekly], gone).slice(-2)).toEqual([
      { value: weekly.id, label: "周报" },
      { value: gone, label: "已删除的预设（按校对处理）" },
    ]);
    expect(outputModeChoices(en, "en").map((c) => c.value)).toEqual([
      "",
      "whole_take",
      "streaming_final",
      "live_inject",
    ]);
    expect(scriptChoices().map((c) => c.label)).toEqual(["跟随全局", "简体", "繁体", "保持原样"]);
    expect(languageChoices("").map((c) => c.value)).toEqual([
      "",
      "auto",
      "zh",
      "en",
      "yue",
      "ja",
      "ko",
    ]);
    // A code the core accepted but the list does not offer stays selectable.
    expect(languageChoices("fr").at(-1)).toEqual({ value: "fr", label: "fr" });
    expect(languageChoices("en")).toHaveLength(7);
    expect(languageName("auto")).toBe("自动检测");
    expect(languageName("ja")).toBe("日本語 · ja");
    expect(languageName("fr")).toBe("fr");
  });

  it("a built-in scene's draft may list no app, and names clash only among the user's scenes (section 18.10)", () => {
    const legal: Scene = {
      ...SCENE,
      id: "b0117e1e-5ce0-4000-8000-000000000004",
      name: "legal",
      builtin: "legal",
    };
    const form = { ...editorDraftFrom(legal), apps: [] };
    expect(editorProblems(form, [SCENE], zhT.t, true)).toEqual({});
    expect(editorProblems(form, [SCENE]).apps).toEqual({ text: "至少添加一个应用", missing: true });
    // A user scene may take a category's name: the built-in one keeps its own.
    const mine = { ...editorDraftFrom(SCENE), name: "legal" };
    expect(editorProblems(mine, [legal]).name).toBeUndefined();
    expect(sceneLabel(legal)).toBe("法律");
    expect(sceneLabel(legal, "en")).toBe("Legal");
    expect(sceneLabel(SCENE, "en")).toBe("Docs");
  });

  it("summarises the overrides a scene sets, in the editor's order and the UI language", () => {
    expect(overrideSummary(SCENE.overrides)).toEqual([
      "AI 润色 开",
      "AI 预设：书面语",
      "输出：整段输出",
      "语言：自动检测",
      "字形：保持原样",
      "有补充要求",
    ]);
    expect(overrideSummary({ refine_enabled: false, language: "en" }, en, "en")).toEqual([
      "AI polish off",
      "Language: English · en",
    ]);
    expect(overrideSummary({})).toEqual([]);
  });
});
