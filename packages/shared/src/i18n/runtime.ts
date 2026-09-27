// Locale resolution, key paths and interpolation. The dictionaries (`zh-CN.ts`, `en.ts`) share one
// nested shape (`Messages`, derived from zh-CN); a leaf is either a plain string with `{name}`
// placeholders or a `{ one, other }` pair picked by the `n` parameter (`{n} 条` vs `{n} entries`).
import type { LocaleSetting } from "../schema";

export const LOCALES = ["zh-CN", "en"] as const;
export type Locale = (typeof LOCALES)[number];

/** A count-sensitive leaf; `one` is used for `n === 1` in English, never in Chinese. */
export interface PluralForms {
  one: string;
  other: string;
}

export type MessageLeaf = string | PluralForms;
export interface MessageTree {
  [key: string]: MessageLeaf | MessageTree;
}

/** Dot-joined paths to every leaf of a message tree (`shell.nav.home`). */
export type LeafPaths<T, Prefix extends string = ""> = {
  [K in keyof T & string]: T[K] extends MessageLeaf
    ? `${Prefix}${K}`
    : T[K] extends MessageTree
      ? LeafPaths<T[K], `${Prefix}${K}.`>
      : never;
}[keyof T & string];

export type Params = Readonly<Record<string, string | number>>;

/** `resolveLocale("system", "zh-CN")` → `zh-CN`; `"system"` follows the OS / webview language and
 *  falls back to English for anything that is not Chinese. */
export function resolveLocale(setting: LocaleSetting, navigatorLanguage: string): Locale {
  switch (setting) {
    case "zh-cn":
      return "zh-CN";
    case "en":
      return "en";
    case "system":
      return navigatorLanguage.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
  }
}

/** BCP 47 tag for `Intl` (`Intl.DateTimeFormat`, `toLocaleString`). */
export function intlTag(locale: Locale): string {
  return locale === "zh-CN" ? "zh-CN" : "en-US";
}

export function isPlural(leaf: MessageLeaf): leaf is PluralForms {
  return typeof leaf !== "string";
}

/** Which plural form a count selects; Chinese has no singular. */
export function pluralForm(locale: Locale, n: number): keyof PluralForms {
  if (locale === "zh-CN") return "other";
  return n === 1 ? "one" : "other";
}

/** `interpolate("{n} 条 · {name}", { n: 3, name: "x" })` → `3 条 · x`; unknown names stay as is. */
export function interpolate(template: string, params: Params | undefined): string {
  if (params === undefined) return template;
  return template.replaceAll(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

/** Walk `tree` along `path`; `undefined` when a segment is missing or lands on a subtree. */
export function lookup(tree: MessageTree, path: string): MessageLeaf | undefined {
  let cursor: MessageLeaf | MessageTree | undefined = tree;
  for (const segment of path.split(".")) {
    if (cursor === undefined || typeof cursor === "string" || isPluralLike(cursor))
      return undefined;
    cursor = cursor[segment];
  }
  if (cursor === undefined || (typeof cursor !== "string" && !isPluralLike(cursor)))
    return undefined;
  return cursor;
}

function isPluralLike(value: MessageTree | PluralForms): value is PluralForms {
  return typeof value.one === "string" && typeof value.other === "string";
}

/** Every leaf path of a tree in declaration order; the dictionaries are compared with this in tests. */
export function leafPaths(tree: MessageTree, prefix = ""): string[] {
  return [...collectLeafPaths(tree, prefix)];
}

function* collectLeafPaths(tree: MessageTree, prefix: string): Generator<string> {
  for (const [key, value] of Object.entries(tree)) {
    const path = `${prefix}${key}`;
    if (typeof value === "string" || isPluralLike(value)) yield path;
    else yield* collectLeafPaths(value, `${path}.`);
  }
}

/** Resolve one message: pick the plural form from `params.n`, then interpolate. A missing key
 *  returns the key itself so a typo is visible on screen instead of blank. */
export function format(
  locale: Locale,
  tree: MessageTree,
  key: string,
  params: Params | undefined,
): string {
  const leaf = lookup(tree, key);
  if (leaf === undefined) return key;
  if (isPlural(leaf)) {
    const n = params?.n;
    const form = typeof n === "number" ? pluralForm(locale, n) : "other";
    return interpolate(leaf[form], params);
  }
  return interpolate(leaf, params);
}
