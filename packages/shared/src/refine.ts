// The token-level diff behind the History page's raw-vs-inserted view. (The rule chain and the
// dictionary matcher run in the core, docs/dictation.md §16; the pages ask it through
// `Backend.vocabularyPreview`.)
import type { SampleHistoryRow } from "./fixtures/history";

export interface DiffSegment {
  kind: "same" | "add" | "del";
  text: string;
}

/** Splits into diff tokens: Latin/digit words (with trailing spaces), single CJK chars, punctuation. */
export function diffTokens(text: string): string[] {
  return text.match(/[A-Za-z0-9_<>@#$%&*+=/\\.:-]+\s*|\s+|[^\sA-Za-z0-9_<>@#$%&*+=/\\.:-]/g) ?? [];
}

/** Token-level LCS diff of `rawText` → `text`, merged into runs (identifiers stay whole). */
export function diffSegments(entry: Pick<SampleHistoryRow, "rawText" | "text">): DiffSegment[] {
  const a = diffTokens(entry.rawText);
  const b = diffTokens(entry.text);
  const table: number[][] = Array.from({ length: a.length + 1 }, () =>
    Array.from({ length: b.length + 1 }, () => 0),
  );
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      const diag = table[i + 1]?.[j + 1] ?? 0;
      const down = table[i + 1]?.[j] ?? 0;
      const right = table[i]?.[j + 1] ?? 0;
      const row = table[i];
      if (row) row[j] = a[i] === b[j] ? diag + 1 : Math.max(down, right);
    }
  }
  const out: DiffSegment[] = [];
  const push = (kind: DiffSegment["kind"], token: string) => {
    const last = out[out.length - 1];
    if (last?.kind === kind) last.text += token;
    else out.push({ kind, text: token });
  };
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    const ta = a[i] ?? "";
    const tb = b[j] ?? "";
    if (ta === tb) {
      push("same", ta);
      i += 1;
      j += 1;
    } else if ((table[i + 1]?.[j] ?? 0) >= (table[i]?.[j + 1] ?? 0)) {
      push("del", ta);
      i += 1;
    } else {
      push("add", tb);
      j += 1;
    }
  }
  while (i < a.length) push("del", a[i++] ?? "");
  while (j < b.length) push("add", b[j++] ?? "");
  return out;
}
