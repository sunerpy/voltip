// Scenes as the core runs them (docs/dictation.md §18), for the in-memory backend and the scene
// editor's local checks: the same limits and refusal texts as `voltip_core::scenes`, the app-id
// normalisation, the first-match rule and `recent_apps`. The desktop app still has the core decide
// (a draft is validated again by the bridge); this module only keeps the mock and the editor honest.
import {
  type AppRef,
  type HistoryEntry,
  type HostOs,
  LANGUAGE_AUTO,
  MAX_APP_ID_CHARS,
  MAX_CONTEXT_NAME_CHARS,
  MAX_CONTEXT_TITLE_CHARS,
  MAX_LANGUAGE_CHARS,
  MAX_RECENT_APPS,
  MAX_SCENE_APPS,
  MAX_SCENE_NAME_CHARS,
  MAX_SCENE_PROMPT_CHARS,
  MAX_SCENES,
  MAX_TITLE_KEYWORD_CHARS,
  MAX_TITLE_KEYWORDS,
  type Scene,
  type SceneDraft,
  type SceneOverrides,
} from "./schema";

/** A refusal carrying the core's text (`scenes: …`). */
export class SceneError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SceneError";
  }
}

const sceneErr = (message: string) => new SceneError(`scenes: ${message}`);

/** Characters (Unicode scalar values), as Rust's `chars().count()`. */
function charCount(text: string): number {
  return Array.from(text).length;
}

/** The id normalisation of §18.3: trim → lower-case → strip trailing `.exe` (repeatedly) → trim. */
export function normalizeAppId(raw: string): string {
  let id = raw.trim().toLowerCase();
  while (id.endsWith(".exe")) id = id.slice(0, -4).trimEnd();
  return id.trim();
}

/** `voltip_platform::foreground::WINDOWS_TERMINALS` (docs/dictation.md §19.2): Windows terminals by
 *  the id the probe reports. */
export const WINDOWS_TERMINALS: readonly string[] = [
  "windowsterminal",
  "cmd",
  "conhost",
  "powershell",
  "pwsh",
  "wezterm-gui",
  "alacritty",
  "mintty",
  "kitty",
  "hyper",
  "tabby",
];

/** `voltip_platform::foreground::LINUX_TERMINALS`: Linux terminals by their X11 `WM_CLASS` class. */
export const LINUX_TERMINALS: readonly string[] = [
  "gnome-terminal-server",
  "gnome-terminal",
  "konsole",
  "xfce4-terminal",
  "xterm",
  "uxterm",
  "urxvt",
  "rxvt",
  "alacritty",
  "kitty",
  "foot",
  "tilix",
  "terminator",
  "wezterm",
  "org.wezfurlong.wezterm",
  "com.mitchellh.ghostty",
  "ghostty",
  "ptyxis",
  "org.gnome.ptyxis",
  "org.gnome.console",
  "kgx",
  "st",
  "st-256color",
  "qterminal",
  "lxterminal",
  "mate-terminal",
  "terminology",
  "yakuake",
  "guake",
  "tilda",
  "cool-retro-term",
  "sakura",
  "deepin-terminal",
];

/** `voltip_platform::foreground::terminal_ids`: the terminals on `os` where a voice edit is refused
 *  (their selection cannot be replaced, and most do not bind the Ctrl+Insert copy); none on macOS
 *  (Cmd+C copies there) or other hosts. */
export function terminalIds(os: HostOs): readonly string[] {
  switch (os) {
    case "windows":
      return WINDOWS_TERMINALS;
    case "linux":
      return LINUX_TERMINALS;
    case "macos":
    case "other":
      return [];
  }
}

/** `voltip_platform::foreground::is_terminal`: the voice edit's guard (§19.2). */
export function isTerminalApp(os: HostOs, appId: string): boolean {
  return terminalIds(os).includes(normalizeAppId(appId));
}

/** A trimmed single-line value of 1..=`max` characters. */
function cleanLine(value: string, what: string, max: number): string {
  const clean = value.trim();
  if (clean.length === 0) throw sceneErr(`${what}不能为空`);
  const chars = charCount(clean);
  if (chars > max) throw sceneErr(`${what}最多 ${max} 个字符（当前 ${chars}）`);
  if (/\p{Cc}/u.test(clean)) throw sceneErr(`${what}不能包含换行或控制字符`);
  return clean;
}

/** Only the overrides that are set, as the core stores them (unset keys absent, not `null`). */
function setOverrides(overrides: SceneOverrides): SceneOverrides {
  const out: SceneOverrides = {};
  if (overrides.refine_enabled != null) out.refine_enabled = overrides.refine_enabled;
  if (overrides.refine_style != null) out.refine_style = overrides.refine_style;
  if (overrides.output_mode != null) out.output_mode = overrides.output_mode;
  if (overrides.language != null) out.language = overrides.language;
  if (overrides.chinese_script != null) out.chinese_script = overrides.chinese_script;
  if (overrides.prompt != null) out.prompt = overrides.prompt;
  return out;
}

/** A draft on its own, normalised as `voltip_core::scenes::validate_scene_draft` does, or a
 *  `scenes: …` refusal with the core's text. */
export function validateSceneDraft(draft: SceneDraft): SceneDraft {
  const name = cleanLine(draft.name, "场景名称", MAX_SCENE_NAME_CHARS);
  const apps: string[] = [];
  for (const raw of draft.match.apps) {
    if (raw.trim().length === 0) continue;
    const normalized = normalizeAppId(raw);
    if (normalized.length === 0) throw sceneErr(`应用 id「${raw.trim()}」无效`);
    const id = cleanLine(normalized, "应用 id", MAX_APP_ID_CHARS);
    if (!apps.includes(id)) apps.push(id);
  }
  if (apps.length === 0) throw sceneErr(`场景「${name}」至少要有一个应用`);
  if (apps.length > MAX_SCENE_APPS)
    throw sceneErr(`一个场景最多 ${MAX_SCENE_APPS} 个应用（当前 ${apps.length}）`);
  const keywords: string[] = [];
  for (const raw of draft.match.title_contains) {
    if (raw.trim().length === 0) continue;
    const keyword = cleanLine(raw, "窗口标题关键词", MAX_TITLE_KEYWORD_CHARS);
    if (!keywords.some((k) => k.toLowerCase() === keyword.toLowerCase())) keywords.push(keyword);
  }
  if (keywords.length > MAX_TITLE_KEYWORDS)
    throw sceneErr(
      `一个场景最多 ${MAX_TITLE_KEYWORDS} 个窗口标题关键词（当前 ${keywords.length}）`,
    );
  const overrides = setOverrides(draft.overrides);
  const language = overrides.language?.trim() ?? "";
  if (language.length === 0) delete overrides.language;
  else {
    const code = language.toLowerCase();
    if (code.length > MAX_LANGUAGE_CHARS || !/^[a-z0-9][a-z0-9-]*$/.test(code))
      throw sceneErr(
        `语言代码「${language}」无效（字母、数字或 -，最多 ${MAX_LANGUAGE_CHARS} 个字符；auto = 自动识别）`,
      );
    overrides.language = code;
  }
  if (overrides.prompt != null) {
    const text = overrides.prompt.replaceAll("\r\n", "\n").replaceAll("\r", "\n").trim();
    if (text.length === 0) delete overrides.prompt;
    else {
      const chars = charCount(text);
      if (chars > MAX_SCENE_PROMPT_CHARS)
        throw sceneErr(`补充要求最多 ${MAX_SCENE_PROMPT_CHARS} 个字符（当前 ${chars}）`);
      // Control characters other than the newline and the tab.
      if (/\p{Cc}/u.test(text.replaceAll("\n", "").replaceAll("\t", "")))
        throw sceneErr("补充要求不能包含控制字符");
      overrides.prompt = text;
    }
  }
  return {
    name,
    enabled: draft.enabled,
    match: { apps, title_contains: keywords },
    overrides,
  };
}

/** A–Z folded to a–z (the core compares scene names ignoring ASCII case only). */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** The whole list: at most `MAX_SCENES`, names unique ignoring ASCII case. */
export function checkScenes(scenes: readonly Scene[]): void {
  if (scenes.length > MAX_SCENES) throw sceneErr(`场景最多 ${MAX_SCENES} 个`);
  scenes.forEach((a, i) => {
    const earlier = scenes.slice(0, i).find((b) => asciiLower(b.name) === asciiLower(a.name));
    if (earlier !== undefined) throw sceneErr(`已有名为「${earlier.name}」的场景`);
  });
}

/** The application in front, as the probe names it. */
export interface ForegroundApp {
  id: string;
  name: string;
  title?: string;
}

/** One line of reference text, as `voltip_core::scenes::clean_context_line`: control characters
 *  become spaces, runs of whitespace collapse, the ends are trimmed and anything past `max`
 *  characters is cut with `…`. `undefined` when nothing is left. */
export function cleanContextLine(raw: string, max: number): string | undefined {
  const words = raw.split(/[\s\p{Cc}]+/u).filter((w) => w.length > 0);
  if (words.length === 0) return undefined;
  const line = words.join(" ");
  const chars = Array.from(line);
  if (chars.length <= max) return line;
  return `${chars
    .slice(0, Math.max(0, max - 1))
    .join("")
    .trimEnd()}…`;
}

/** The probe's answer as the core keeps it (`ForegroundApp::sanitized`, §18.2): the id normalised,
 *  the name one clean line (the id when empty), the title one clean line or absent; `undefined`
 *  when no id is left — that take has no context. */
export function sanitizeForegroundApp(app: ForegroundApp): ForegroundApp | undefined {
  const id = normalizeAppId(app.id);
  if (id.length === 0) return undefined;
  const name = cleanContextLine(app.name, MAX_CONTEXT_NAME_CHARS) ?? id;
  const title =
    app.title === undefined ? undefined : cleanContextLine(app.title, MAX_CONTEXT_TITLE_CHARS);
  return title === undefined ? { id, name } : { id, name, title };
}

/** §18.3: the first enabled scene, in list order, listing the app and — when it has title
 *  keywords — one the window title contains (case-insensitive). */
export function matchScene(scenes: readonly Scene[], app: ForegroundApp): Scene | undefined {
  const id = normalizeAppId(app.id);
  if (id.length === 0) return undefined;
  const title = app.title?.toLowerCase();
  return scenes.find(
    (s) =>
      s.enabled &&
      s.match.apps.some((a) => normalizeAppId(a) === id) &&
      (s.match.title_contains.length === 0 ||
        (title !== undefined &&
          s.match.title_contains.some((k) => k.length > 0 && title.includes(k.toLowerCase())))),
  );
}

/** `recent_apps`: the apps the history saw, newest first, one per id, at most `limit`. */
export function recentApps(
  history: readonly HistoryEntry[],
  limit: number = MAX_RECENT_APPS,
): AppRef[] {
  const out: AppRef[] = [];
  for (const entry of history) {
    if (out.length === limit) break;
    const app = entry.app;
    if (app !== undefined && !out.some((a) => a.id === app.id)) out.push({ ...app });
  }
  return out;
}

/** `true` when the language override means "no hint" (auto-detect). */
export function isAutoLanguage(language: string | null | undefined): boolean {
  return language === LANGUAGE_AUTO;
}
