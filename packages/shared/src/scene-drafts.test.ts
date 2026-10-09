import { createTranslator, zhT } from "./i18n";
import {
  addSceneApp,
  addSceneKeyword,
  hasSceneProblems,
  languageName,
  sceneDraftOf,
  sceneEditorDraftFrom,
  sceneEditorProblems,
  sceneLanguageChoices,
  sceneOutputModeChoices,
  sceneOverrideSummary,
  scenePresetChoices,
  scenePromptChars,
  sceneRefineChoices,
  sceneScriptChoices,
  sceneWithEnabled,
} from "./scene-drafts";
import { sceneLabel } from "./labels";
import { MAX_SCENE_APPS, MAX_TITLE_KEYWORDS, type Scene } from "./schema";

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

describe("the scene editor helpers, desktop and phone (docs/dictation.md section 18)", () => {
  it("round-trips a scene through the form and back to the wire draft", () => {
    const form = sceneEditorDraftFrom(SCENE);
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
      sceneDraftOf({ ...sceneEditorDraftFrom(), name: " New ", language: " ", prompt: " \n" }),
    ).toEqual({
      name: "New",
      enabled: true,
      match: { apps: [], title_contains: [] },
      overrides: {},
    });
    expect(sceneEditorDraftFrom({ ...SCENE, overrides: { refine_enabled: false } }).refine).toBe(
      "off",
    );
    expect(sceneWithEnabled(SCENE, false)).toEqual({
      name: "Docs",
      enabled: false,
      match: SCENE.match,
      overrides: SCENE.overrides,
    });
  });

  it("adds normalised app ids and keywords once", () => {
    expect(addSceneApp([], " Slack.EXE ")).toEqual(["slack"]);
    expect(addSceneApp(["slack"], "SLACK")).toEqual(["slack"]);
    expect(addSceneApp(["slack"], ".exe")).toEqual(["slack"]);
    expect(addSceneKeyword([], " GitHub ")).toEqual(["GitHub"]);
    expect(addSceneKeyword(["GitHub"], "github")).toEqual(["GitHub"]);
    expect(addSceneKeyword(["GitHub"], "  ")).toEqual(["GitHub"]);
    expect(scenePromptChars(" 第一行\r\n第二行 ")).toBe(7);
  });

  it("reports the problems the core would, per field, marking the missing ones", () => {
    const ok = { ...sceneEditorDraftFrom(), name: "a", apps: ["x"] };
    expect(hasSceneProblems(sceneEditorProblems(ok, []))).toBe(false);
    const empty = sceneEditorProblems(sceneEditorDraftFrom(), []);
    expect(empty.name).toEqual({ text: "请填写名称", missing: true });
    expect(empty.apps).toEqual({ text: "至少添加一个应用", missing: true });
    const many = sceneEditorProblems(
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
    expect(sceneEditorProblems({ ...ok, name: "DOCS" }, [SCENE]).name).toEqual({
      text: "已有名为「Docs」的场景",
      missing: false,
    });
  });

  it("offers 跟随全局 first in every select and names languages like the engines dialog", () => {
    for (const choices of [
      sceneRefineChoices(),
      scenePresetChoices([], ""),
      sceneOutputModeChoices(),
      sceneScriptChoices(),
    ])
      expect(choices[0]).toEqual({ value: "", label: "跟随全局" });
    expect(scenePresetChoices([], "", en).map((c) => c.label)).toEqual([
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
    expect(scenePresetChoices([weekly], weekly.id).slice(-1)).toEqual([
      { value: weekly.id, label: "周报" },
    ]);
    const gone = "11111111-1111-4111-8111-111111111111";
    expect(scenePresetChoices([weekly], gone).slice(-2)).toEqual([
      { value: weekly.id, label: "周报" },
      { value: gone, label: "已删除的预设（按校对处理）" },
    ]);
    expect(sceneOutputModeChoices(en, "en").map((c) => c.value)).toEqual([
      "",
      "whole_take",
      "streaming_final",
      "live_inject",
    ]);
    expect(sceneScriptChoices().map((c) => c.label)).toEqual([
      "跟随全局",
      "简体",
      "繁体",
      "保持原样",
    ]);
    expect(sceneLanguageChoices("").map((c) => c.value)).toEqual([
      "",
      "auto",
      "zh",
      "en",
      "yue",
      "ja",
      "ko",
    ]);
    // A code the core accepted but the list does not offer stays selectable.
    expect(sceneLanguageChoices("fr").at(-1)).toEqual({ value: "fr", label: "fr" });
    expect(sceneLanguageChoices("en")).toHaveLength(7);
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
    const form = { ...sceneEditorDraftFrom(legal), apps: [] };
    expect(sceneEditorProblems(form, [SCENE], zhT.t, { builtin: true })).toEqual({});
    expect(sceneEditorProblems(form, [SCENE]).apps).toEqual({
      text: "至少添加一个应用",
      missing: true,
    });
    // A user scene may take a category's name: the built-in one keeps its own.
    const mine = { ...sceneEditorDraftFrom(SCENE), name: "legal" };
    expect(sceneEditorProblems(mine, [legal]).name).toBeUndefined();
    expect(sceneLabel(legal)).toBe("法律");
    expect(sceneLabel(legal, "en")).toBe("Legal");
    expect(sceneLabel(SCENE, "en")).toBe("Docs");
  });

  it("on the phone a scene is picked by hand, so it needs no application (user decision 2026-10-01)", () => {
    const form = { ...sceneEditorDraftFrom(), name: "会议" };
    expect(sceneEditorProblems(form, [SCENE], zhT.t, { needApps: false })).toEqual({});
    expect(sceneEditorProblems(form, [SCENE]).apps?.missing).toBe(true);
    // The other checks stay: a name, unique among the user's scenes.
    const unnamed = sceneEditorProblems(sceneEditorDraftFrom(), [], zhT.t, { needApps: false });
    expect(unnamed).toEqual({ name: { text: "请填写名称", missing: true } });
    const clash = sceneEditorProblems({ ...form, name: "docs" }, [SCENE], en, { needApps: false });
    expect(clash.name?.missing).toBe(false);
    // The wire draft of such a scene matches nothing.
    expect(sceneDraftOf(form).match).toEqual({ apps: [], title_contains: [] });
  });

  it("summarises the overrides a scene sets, in the editor's order and the UI language", () => {
    expect(sceneOverrideSummary(SCENE.overrides)).toEqual([
      "AI 润色 开",
      "AI 预设：书面语",
      "输出：整段输出",
      "语言：自动检测",
      "字形：保持原样",
      "有补充要求",
    ]);
    expect(sceneOverrideSummary({ refine_enabled: false, language: "en" }, en, "en")).toEqual([
      "AI polish off",
      "Language: English · en",
    ]);
    expect(sceneOverrideSummary({})).toEqual([]);
  });
});
