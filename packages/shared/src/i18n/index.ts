// Public i18n surface: dictionaries, `MessageKey`, `createTranslator(locale)` and the plain
// `translate(locale, key, params)` used by the label helpers in `labels.ts`.
import { en } from "./en";
import { type LeafPaths, type Locale, type Params, format, intlTag } from "./runtime";
import { type Messages, zhCN } from "./zh-CN";

export * from "./runtime";
export { en } from "./en";
export { type Messages, zhCN } from "./zh-CN";

export const MESSAGES: Readonly<Record<Locale, Messages>> = { "zh-CN": zhCN, en };

/** Every leaf key of the dictionary, dot-joined (`shell.nav.home`). */
export type MessageKey = LeafPaths<Messages>;

/** `t("count.entries", { n: 3 })` → `3 条` / `3 entries`. */
export type TFunction = (key: MessageKey, params?: Params) => string;

export interface Translator {
  locale: Locale;
  /** BCP 47 tag for `Intl` formatters. */
  tag: string;
  t: TFunction;
  messages: Messages;
}

export function translate(locale: Locale, key: MessageKey, params?: Params): string {
  return format(locale, MESSAGES[locale], key, params);
}

const cache = new Map<Locale, Translator>();

/** Translator for one locale; cached so React deps stay referentially stable. */
export function createTranslator(locale: Locale): Translator {
  const cached = cache.get(locale);
  if (cached) return cached;
  const translator: Translator = {
    locale,
    tag: intlTag(locale),
    t: (key, params) => translate(locale, key, params),
    messages: MESSAGES[locale],
  };
  cache.set(locale, translator);
  return translator;
}

/** The default translator before any provider mounts (and for callers without a locale). */
export const DEFAULT_LOCALE: Locale = "zh-CN";
export const zhT: Translator = createTranslator(DEFAULT_LOCALE);

/** `Intl.DateTimeFormat` in the locale; `Date` style helpers the pages share. */
export function formatDateTime(
  locale: Locale,
  ms: number,
  options: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat(intlTag(locale), options).format(new Date(ms));
}
