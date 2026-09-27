import { describe, expect, it } from "vitest";
import { ONBOARDING_DONE_KEY, firstRunPath } from "./first-run";

function storage(entries: Record<string, string> = {}): Pick<Storage, "getItem"> {
  return { getItem: (key) => entries[key] ?? null };
}

describe("first run", () => {
  it("regression: a launch at the home page opens the first-run guide until it is finished or skipped", () => {
    expect(firstRunPath("/", storage())).toBe("/onboarding");
    expect(firstRunPath("/", storage({ [ONBOARDING_DONE_KEY]: "1" }))).toBeUndefined();
  });

  it("never takes over another window or a deep link", () => {
    expect(firstRunPath("/overlay", storage())).toBeUndefined();
    expect(firstRunPath("/settings/engine", storage())).toBeUndefined();
    expect(firstRunPath("/onboarding?step=3", storage())).toBeUndefined();
    // No storage (it cannot remember the answer): never force the guide on every launch.
    expect(firstRunPath("/", undefined)).toBeUndefined();
  });
});
