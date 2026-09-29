import { dictationFailureText, dictationPhaseLabel } from "../labels";
import {
  DEFAULT_LOCALE,
  LOCALES,
  MESSAGES,
  type MessageTree,
  createTranslator,
  en,
  format,
  formatDateTime,
  interpolate,
  intlTag,
  leafPaths,
  lookup,
  pluralForm,
  resolveLocale,
  translate,
  zhCN,
  zhT,
} from "./index";

const CJK = /[一-鿿]/;

describe("i18n dictionaries", () => {
  it("regression: every locale dictionary has the same key set", () => {
    const reference = leafPaths(zhCN);
    expect(reference.length).toBeGreaterThan(400);
    expect(new Set(reference).size).toBe(reference.length);
    for (const locale of LOCALES) {
      const paths = leafPaths(MESSAGES[locale]);
      expect(paths).toHaveLength(reference.length);
      expect(new Set(paths)).toEqual(new Set(reference));
    }
  });

  it("the English dictionary carries no CJK text except the language names themselves", () => {
    const allowed = new Set([
      "settings.general.locale.zh-cn",
      "language.zh",
      "language.yue",
      "language.ja",
      "language.ko",
    ]);
    const tree: MessageTree = en;
    const offenders = leafPaths(tree).filter((path) => {
      if (allowed.has(path)) return false;
      const leaf = lookup(tree, path);
      const text = typeof leaf === "string" ? leaf : `${leaf?.one ?? ""}${leaf?.other ?? ""}`;
      return CJK.test(text);
    });
    expect(offenders).toEqual([]);
  });

  it("every plural leaf has both non-empty forms in both locales", () => {
    for (const locale of LOCALES) {
      const tree: MessageTree = MESSAGES[locale];
      const leaves = leafPaths(tree).map((path) => lookup(tree, path));
      expect(leaves.filter((leaf) => leaf === undefined)).toEqual([]);
      const plurals = leaves.filter((leaf) => leaf !== undefined && typeof leaf !== "string");
      expect(plurals.length).toBeGreaterThan(3);
      expect(
        plurals.filter(
          (leaf) => typeof leaf === "string" || leaf?.one.length === 0 || leaf?.other.length === 0,
        ),
      ).toEqual([]);
    }
  });
});

describe("resolveLocale", () => {
  it("follows the explicit setting and the browser language for system", () => {
    expect(resolveLocale("zh-cn", "en-US")).toBe("zh-CN");
    expect(resolveLocale("en", "zh-CN")).toBe("en");
    expect(resolveLocale("system", "zh-CN")).toBe("zh-CN");
    expect(resolveLocale("system", "ZH-Hant-TW")).toBe("zh-CN");
    expect(resolveLocale("system", "en-GB")).toBe("en");
    expect(resolveLocale("system", "")).toBe("en");
    expect(intlTag("zh-CN")).toBe("zh-CN");
    expect(intlTag("en")).toBe("en-US");
  });
});

describe("format", () => {
  it("interpolates named parameters and leaves unknown ones visible", () => {
    expect(interpolate("{n} 条 · {name}", { n: 3, name: "x" })).toBe("3 条 · x");
    expect(interpolate("{n} 条", undefined)).toBe("{n} 条");
    expect(interpolate("{missing}", { n: 1 })).toBe("{missing}");
  });

  it("picks the plural form by locale and count", () => {
    expect(pluralForm("zh-CN", 1)).toBe("other");
    expect(pluralForm("en", 1)).toBe("one");
    expect(pluralForm("en", 2)).toBe("other");
    expect(translate("en", "count.entries", { n: 1 })).toBe("1 entry");
    expect(translate("en", "count.entries", { n: 2 })).toBe("2 entries");
    expect(translate("zh-CN", "count.entries", { n: 1 })).toBe("1 条");
    // A plural key without a count falls back to `other`.
    expect(translate("en", "count.entries")).toBe("{n} entries");
  });

  it("returns the key itself for a missing or non-leaf path", () => {
    const tree: MessageTree = zhCN;
    expect(format("zh-CN", tree, "nope.nothing", undefined)).toBe("nope.nothing");
    expect(format("zh-CN", tree, "shell.nav", undefined)).toBe("shell.nav");
    expect(lookup(tree, "shell.nav.home.deeper")).toBeUndefined();
    expect(lookup(tree, "count.entries.one")).toBeUndefined();
  });

  it("createTranslator caches per locale and exposes the Intl tag", () => {
    const a = createTranslator("en");
    expect(createTranslator("en")).toBe(a);
    expect(a.tag).toBe("en-US");
    expect(a.messages).toBe(en);
    expect(a.t("shell.nav.home")).toBe("Home");
    expect(zhT.locale).toBe(DEFAULT_LOCALE);
    expect(zhT.t("shell.nav.home")).toBe("首页");
  });

  it("formatDateTime uses the locale's Intl formatter", () => {
    const ms = Date.UTC(2026, 8, 24, 12, 0, 0);
    expect(formatDateTime("en", ms, { month: "short", timeZone: "UTC" })).toBe("Sep");
    expect(formatDateTime("zh-CN", ms, { month: "numeric", timeZone: "UTC" })).toBe("9月");
  });
});

describe("dictation failure codes", () => {
  it("regression: dictation failure codes are localized", () => {
    const failed = { phase: "failed" as const, message: "asr: 503", code: "no_speech" as const };
    expect(dictationFailureText(failed, "zh-CN")).toBe("没有听到声音");
    expect(dictationFailureText(failed, "en")).toBe("No speech detected");
    expect(dictationPhaseLabel(failed, 0, "en").text).toBe("Failed · No speech detected");
    expect(dictationPhaseLabel({ ...failed, text: "kept" }, 0, "zh-CN").text).toBe(
      "没有送出 · 没有听到声音",
    );
    for (const code of ["audio", "asr", "refine", "inject"] as const) {
      expect(dictationFailureText({ ...failed, code }, "en")).not.toMatch(CJK);
      expect(dictationFailureText({ ...failed, code }, "zh-CN")).toMatch(CJK);
    }
    // `unknown` and a missing code fall back to the core's own message.
    expect(dictationFailureText({ ...failed, code: "unknown" }, "en")).toBe("asr: 503");
    expect(dictationFailureText({ phase: "failed", message: "boom" }, "en")).toBe("boom");
  });
});
