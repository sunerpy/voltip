// `@voltip/ui`'s I18nProvider for React Native (copied for the same reason as BackendProvider).
// The dictionaries are `@voltip/shared`'s.
import { type Locale, type Translator, createTranslator, zhT } from "@voltip/shared";
import { type ReactNode, createContext, useContext, useMemo } from "react";

const I18nContext = createContext<Translator>(zhT);

export function I18nProvider({ locale, children }: { locale: Locale; children: ReactNode }) {
  const translator = useMemo(() => createTranslator(locale), [locale]);
  return <I18nContext.Provider value={translator}>{children}</I18nContext.Provider>;
}

/** The whole translator: `t`, `locale`, `tag` (for `Intl`), `messages`. */
export function useI18n(): Translator {
  return useContext(I18nContext);
}

/** `t(key, params)` for the mounted locale (zh-CN without a provider). */
export function useT(): Translator["t"] {
  return useContext(I18nContext).t;
}
