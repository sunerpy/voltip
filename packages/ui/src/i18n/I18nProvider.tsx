import { type Locale, type Translator, createTranslator, zhT } from "@voltip/shared";
import { type ReactNode, createContext, useContext, useEffect } from "react";

const I18nContext = createContext<Translator>(zhT);

export interface I18nProviderProps {
  locale: Locale;
  children: ReactNode;
  /** Mirror the locale onto `<html lang>` (the app shells; component tests leave it alone). */
  documentLang?: boolean;
}

/** Shares one translator with every component; without a provider everything renders zh-CN, so
 *  components keep their default wording (and the existing tests keep passing). */
export function I18nProvider({ locale, children, documentLang = false }: I18nProviderProps) {
  const translator = createTranslator(locale);
  useEffect(() => {
    if (!documentLang) return;
    document.documentElement.lang = translator.tag;
  }, [documentLang, translator]);
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

export function useLocale(): Locale {
  return useContext(I18nContext).locale;
}
