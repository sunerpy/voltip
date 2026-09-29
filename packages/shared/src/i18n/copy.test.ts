// Interface copy guard (docs/frontend.md §8「界面文案」): no leaf of either dictionary uses the
// internal words the interface used to leak (「核心」「热键后端」「边沿」「注入」「LLM」…), and none
// slips into colloquial wording (「还没」「没能」, "just"): the register is standard, readable product
// text. Two kinds of keys are exempt: the overlay spec sheet's (`/overlay`, loaded in dev builds
// only; this test checks that no release module reads them) and the technical-details lines, which
// alone may name the cryptography.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { LOCALES, MESSAGES, type MessageTree, leafPaths, lookup } from "./index";

/** Words the Chinese interface does not use, each with the word that replaces it. */
const ZH_JARGON: ReadonlyArray<readonly [RegExp, string]> = [
  [/核心/, "（不出现）"],
  [/后端/, "快捷键方式"],
  [/边沿/, "按下 / 松开"],
  [/注入|投递/, "送出 / 粘贴到光标处"],
  [/(?<![A-Za-z])LLM/, "AI"],
  [/(?<!Qwen3-)ASR/, "语音识别"],
  [/握手/, "建立加密连接"],
  [/对端/, "对方设备"],
  [/票据/, "配对信息"],
  [/枚举/, "查找"],
  [/持久化/, "保存"],
  [/TTL/, "有效期"],
  [/(?<!Silero )VAD/, "静音检测"],
  [/UIPI|提权/, "以管理员身份运行的窗口"],
  [/状态机/, "（不出现）"],
  [/热键/, "快捷键"],
  [/收音/, "录音"],
  [/\.json\b/, "这台电脑上"],
];

/** The same list for the English interface. */
const EN_JARGON: ReadonlyArray<readonly [RegExp, string]> = [
  [/\bcore\b/i, "(nothing)"],
  [/\bbackends?\b/i, "shortcut method"],
  [/\bedges?\b/i, "press / release"],
  [/\binject/i, "send / paste at the cursor"],
  [/(?<![A-Za-z])LLMs?\b/, "AI"],
  [/(?<!Qwen3-)\bASR\b/, "speech recognition / transcription"],
  [/handshake/i, "encrypted connection"],
  [/\bpeers?\b/i, "the other device"],
  [/\btickets?\b/i, "pairing info"],
  [/enumerat/i, "look for"],
  [/persist/i, "save"],
  [/\bTTL\b/, "valid for"],
  [/(?<!Silero )\bVAD\b/, "silence detection"],
  [/UIPI|\belevated\b/i, "runs as administrator"],
  [/state machine/i, "(nothing)"],
  [/\bhotkeys?\b/i, "shortcut"],
  [/\bchords?\b/i, "shortcut / key combination"],
  [/\.json\b/, "on this computer"],
];

/** Colloquial wording the standard register avoids (user decision 2026-09-29: 规范、易读，不用
 *  大白话), with the written form to use instead. */
const ZH_COLLOQUIAL: ReadonlyArray<readonly [RegExp, string]> = [
  [/还没/, "尚未 / 未"],
  [/没能/, "未能 / 无法"],
  [/免得/, "以免"],
  [/搭的|咋|啥/, "（书面说法）"],
];
const EN_COLLOQUIAL: ReadonlyArray<readonly [RegExp, string]> = [
  // "Just now" is the standard relative-time label.
  [/\bjust\b(?! now)/i, "(cut it)"],
  [/\bgonna\b|\bstuff\b/i, "(written form)"],
];

/** Cipher and protocol names: only in a technical-details line. */
const CRYPTO = /X25519|Ed25519|ChaCha20|Poly1305|HKDF|Noise XX|AES-GCM|SHA-?256/i;

/** The lines the interface shows under 技术细节 / Technical details. */
const TECHNICAL_KEYS: ReadonlySet<string> = new Set(["devices.syncPanel.crypto"]);

/** Keys the overlay spec sheet alone reads (`OverlaySheet.tsx`, `import.meta.env.DEV` only): design
 *  notes and sample values, exempt from the word list. A trailing dot is a whole subtree. */
const SPEC_SHEET_ONLY: readonly string[] = [
  "overlay.heading",
  "overlay.label.",
  "overlay.fallbackText",
  "overlay.captionEngine",
  "overlay.captionCommitted",
  "overlay.captionTail",
  "overlay.expanded",
  "overlay.anatomy",
  "overlay.anatomyBody",
  "overlay.toastTitle",
  "overlay.toast.",
  "overlay.fallback.",
  "overlay.failures.",
  "overlay.causes.",
];

function specSheetOnly(path: string): boolean {
  return SPEC_SHEET_ONLY.some((key) => (key.endsWith(".") ? path.startsWith(key) : path === key));
}

function leafText(tree: MessageTree, path: string): string {
  const leaf = lookup(tree, path);
  return typeof leaf === "string" ? leaf : `${leaf?.one ?? ""} ${leaf?.other ?? ""}`;
}

/** The words a reader sees: `{hotkey}` is filled in by the code, not read as the word. */
function shownWords(text: string): string {
  return text.replaceAll(/\{\w+\}/g, "");
}

const repo = resolve(import.meta.dirname, "../../../..");

/** Every TypeScript module that ships: the web sources minus tests, the spec sheet and the
 *  dictionaries themselves. */
function releaseSources(): string[] {
  const roots = ["apps/desktop/src", "apps/mobile/src", "packages/ui/src", "packages/shared/src"];
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        if (name !== "node_modules" && name !== "fixtures") walk(path);
      } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) {
        out.push(path);
      }
    }
  };
  for (const root of roots) walk(resolve(repo, root));
  return out.filter(
    (path) => !path.endsWith("OverlaySheet.tsx") && !/i18n[\\/](zh-CN|en)\.ts$/.test(path),
  );
}

describe("interface copy", () => {
  it("regression: user-facing strings use standard, readable words, not internal jargon or colloquial speech", () => {
    const offenders: string[] = [];
    for (const locale of LOCALES) {
      const tree: MessageTree = MESSAGES[locale];
      const words =
        locale === "zh-CN" ? [...ZH_JARGON, ...ZH_COLLOQUIAL] : [...EN_JARGON, ...EN_COLLOQUIAL];
      for (const path of leafPaths(tree)) {
        if (specSheetOnly(path)) continue;
        const text = shownWords(leafText(tree, path));
        for (const [word, instead] of words) {
          if (word.test(text)) offenders.push(`${locale} ${path}: ${word.source} → ${instead}`);
        }
        if (CRYPTO.test(text) && !TECHNICAL_KEYS.has(path)) {
          offenders.push(`${locale} ${path}: cipher names belong in a technical-details line`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it("the technical-details lines say so", () => {
    for (const path of TECHNICAL_KEYS) {
      expect(leafText(MESSAGES["zh-CN"], path)).toMatch(/^技术细节：/);
      expect(leafText(MESSAGES.en, path)).toMatch(/^Technical details: /);
    }
  });

  it("the exempt spec-sheet keys exist and no release module reads them", () => {
    const paths = leafPaths(MESSAGES["zh-CN"]);
    for (const key of SPEC_SHEET_ONLY) {
      expect(paths.some((path) => (key.endsWith(".") ? path.startsWith(key) : path === key))).toBe(
        true,
      );
    }
    const readers: string[] = [];
    for (const file of releaseSources()) {
      const source = readFileSync(file, "utf8");
      for (const key of SPEC_SHEET_ONLY) {
        // `overlay.anatomy` is a prefix of `overlay.anatomyBody`: match a whole quoted key.
        const pattern = key.endsWith(".")
          ? new RegExp(`["'\`]${key.replaceAll(".", "\\.")}`)
          : new RegExp(`["'\`]${key.replaceAll(".", "\\.")}["'\`]`);
        if (pattern.test(source)) readers.push(`${file.slice(repo.length + 1)} reads ${key}`);
      }
    }
    expect(readers).toEqual([]);
  });
});
