// The clipboard fallback codes (docs/dictation.md §4.2): every code the core can send has its
// sentence in both interface languages, and each sentence names the paste keys.
import { LOCALES, MESSAGES, lookup } from "./i18n";
import { CLIPBOARD_CODES, clipboardCodeSchema, historyOutcomeSchema } from "./schema";

describe("clipboard fallback codes", () => {
  it("every code has a sentence in both languages that names the paste keys", () => {
    for (const locale of LOCALES) {
      for (const code of CLIPBOARD_CODES) {
        const text = lookup(MESSAGES[locale], `history.clipboardNote.${code}`);
        expect(typeof text, `${locale} ${code}`).toBe("string");
        expect(text, `${locale} ${code}`).toContain("{keys}");
      }
    }
  });

  it("an outcome carries its code, and one written before the codes parses without it", () => {
    expect(
      historyOutcomeSchema.parse({ kind: "clipboard", reason: "x", code: "no_permission" }),
    ).toEqual({ kind: "clipboard", reason: "x", code: "no_permission" });
    expect(historyOutcomeSchema.parse({ kind: "clipboard", reason: "x" })).toEqual({
      kind: "clipboard",
      reason: "x",
    });
    expect(clipboardCodeSchema.safeParse("elevated").success).toBe(false);
  });
});
