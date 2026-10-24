import { PROVIDER_MARKS } from "./provider-marks";
import { PROVIDER_IDS } from "./schema";

describe("provider marks (docs/dictation.md §3)", () => {
  it("every vendor has a mark; the built-in, on-device and custom providers keep the app's icons", () => {
    expect(PROVIDER_IDS.filter((id) => PROVIDER_MARKS[id] === undefined)).toEqual([
      "builtin",
      "local",
      "custom",
    ]);
  });

  it("each mark is plain path data on the 24×24 grid, coloured or taking the text colour", () => {
    const paths = Object.values(PROVIDER_MARKS).flatMap((mark) => mark.paths);
    expect(paths.length).toBeGreaterThan(0);
    // Path commands and numbers only: nothing that could carry markup or a URL.
    expect(paths.filter((p) => !/^[MmLlHhVvCcSsQqTtAaZz0-9.,\s\-e]+$/.test(p.d))).toEqual([]);
    expect(
      paths
        .flatMap((p) => (p.fill === undefined ? [] : [p.fill]))
        .filter((f) => !/^#[0-9A-F]{6}$/.test(f)),
    ).toEqual([]);
    // The black-and-white marks follow the theme; the coloured ones keep their colours.
    const mono = Object.entries(PROVIDER_MARKS)
      .filter(([, mark]) => mark.paths.every((p) => p.fill === undefined))
      .map(([id]) => id);
    mono.sort();
    expect(mono).toEqual(["google", "groq", "kimi", "mimo", "ollama", "openai", "openrouter"]);
  });
});
