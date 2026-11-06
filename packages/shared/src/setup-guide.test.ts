// docs/dictation.md §3.16: a provider's 配置方法 in the drawer and its section of the guide's
// 服务商设置 page go together. Every provider but the built-in service and this device has steps in
// both languages, a label for the button of its setup page when it has one, and a section on the
// page in both languages under the anchor the shell opens (`voltip_core::ui::provider_anchor`).
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { setupPageLabel, setupSteps } from "./engine-drafts";
import { LOCALES, MESSAGES } from "./i18n";
import { providerSpec } from "./providers";
import { PROVIDER_IDS, type ProviderId } from "./schema";

const repo = resolve(import.meta.dirname, "../../..");
const PAGES = {
  "zh-CN": "docs/site/zh/recognition/providers.md",
  en: "docs/site/recognition/providers.md",
} as const;
/** Nothing to set up: the built-in service comes with the app, this device's models are cards of
 *  their own. */
const SETTLED: readonly ProviderId[] = ["builtin", "local"];
const vendors = PROVIDER_IDS.filter((id) => !SETTLED.includes(id));

/** The anchor `provider_anchor` gives `id`. */
const anchor = (id: ProviderId) => id.replaceAll("_", "-");

describe("setup guide (docs/dictation.md §3.16)", () => {
  it("gives every provider to set up its steps, as many in each language", () => {
    // Each list names the providers that break the rule, so a failure says which.
    const uneven = vendors.filter((id) => {
      const counts = LOCALES.map((locale) => setupSteps(id, MESSAGES[locale]).length);
      return new Set(counts).size !== 1 || counts.some((n) => n < 2 || n > 4);
    });
    expect(uneven).toEqual([]);
    const settled = SETTLED.filter((id) =>
      LOCALES.some((locale) => setupSteps(id, MESSAGES[locale]).length > 0),
    );
    expect(settled).toEqual([]);
  });

  it("names what each setup page is for, and only providers with one have a label", () => {
    const withPage = PROVIDER_IDS.filter((id) => providerSpec(id).setup === true);
    for (const locale of LOCALES) {
      const labels: Partial<Record<ProviderId, string>> = MESSAGES[locale].engines.setup.page;
      expect(PROVIDER_IDS.filter((id) => labels[id] !== undefined)).toEqual(withPage);
      expect(setupPageLabel("custom", MESSAGES[locale])).toBe(
        MESSAGES[locale].engines.setup.console,
      );
    }
  });

  it("has a section for every provider on the providers page, in both languages", () => {
    for (const locale of LOCALES) {
      const page = readFileSync(resolve(repo, PAGES[locale]), "utf8");
      const headings = page.split("\n").filter((line) => line.startsWith("## "));
      const unmarked = vendors.filter(
        (id) => headings.filter((line) => line.trimEnd().endsWith(`{#${anchor(id)}}`)).length !== 1,
      );
      expect(unmarked).toEqual([]);
      // Each section's anchor is a provider's: no stray ones that the app could not open.
      const anchors = headings.flatMap((line) => /\{#([a-z0-9-]+)\}\s*$/.exec(line)?.[1] ?? []);
      expect(new Set(anchors)).toEqual(new Set(vendors.map(anchor)));
      expect(anchors).toHaveLength(vendors.length);
    }
  });
});
