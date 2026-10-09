// User feedback 2026-09-29 (plan 1.6): text that fits the window stays on one line. No caller gives
// a Select a fixed width (the component sizes itself from its longest option), and the caps that
// wrapped help text and readouts early stay gone.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const ROOTS = [resolve(__dirname, ".."), resolve(__dirname, "../../../mobile/src")];

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return name.endsWith(".tsx") && !name.endsWith(".test.tsx") ? [path] : [];
  });
}

/** The props text of every `<Select …>` element in `source`. */
function selectProps(source: string): string[] {
  return [...source.matchAll(/<Select(?:<[^>]*>)?\s([\s\S]*?)\/>/g)].map((m) => m[1] ?? "");
}

describe("fixed widths", () => {
  it("regression: no caller gives a Select a fixed width", () => {
    const offenders = ROOTS.flatMap((root) =>
      sources(root).flatMap((file) =>
        selectProps(readFileSync(file, "utf8"))
          .filter((props) => /className="[^"]*\bw-(?:\d|\[)/.test(props))
          .map(() => relative(resolve(__dirname, "../../.."), file)),
      ),
    );
    expect(offenders).toEqual([]);
  });

  it("regression: the settings help and readouts have no fixed cap", () => {
    const capped = ROOTS.flatMap((root) =>
      sources(root).flatMap((file) => {
        const text = readFileSync(file, "utf8");
        return ["max-w-[280px]", "max-w-[320px]", "max-w-[360px]", "max-w-[720px]"]
          .filter((cap) => text.includes(cap))
          .map((cap) => `${relative(resolve(__dirname, "../../.."), file)}: ${cap}`);
      }),
    );
    expect(capped).toEqual([]);
  });
});
