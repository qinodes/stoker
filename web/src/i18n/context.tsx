import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { isLocale, LANGUAGE_STORAGE_KEY, resolveLocale, translate, translateMessage, translateState, type Locale, type Translator, type UiMessage } from "./messages.ts";

function initialLocale(): Locale {
  let saved: string | null = null;
  try { saved = localStorage.getItem(LANGUAGE_STORAGE_KEY); } catch { /* Browser storage may be unavailable. */ }
  return resolveLocale(saved, navigator.languages?.length ? navigator.languages : [navigator.language]);
}

interface I18nValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: Translator;
  stateLabel: (state?: string | null) => string;
  renderMessage: (message: UiMessage) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, updateLocale] = useState(initialLocale);
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
  const value = useMemo<I18nValue>(() => ({
    locale,
    setLocale: (next) => {
      if (!isLocale(next)) return;
      updateLocale(next);
      try { localStorage.setItem(LANGUAGE_STORAGE_KEY, next); } catch { /* Keep switching functional without storage. */ }
    },
    t: (key, ...args) => translate(locale, key, ...args),
    stateLabel: (state) => translateState(locale, state),
    renderMessage: (message) => translateMessage(locale, message),
  }), [locale]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n must be used inside I18nProvider");
  return value;
}

export function LanguagePicker() {
  const { locale, setLocale, t } = useI18n();
  return <span className="language-control">
    <svg className="language-globe" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true" focusable="false">
      <circle cx="12" cy="12" r="9" />
      <ellipse cx="12" cy="12" rx="4" ry="9" />
      <path d="M3 12h18" />
    </svg>
    <select className="language-picker" aria-label={t("language.label")} title={t("language.label")} value={locale} onChange={(event) => {
      if (isLocale(event.target.value)) setLocale(event.target.value);
    }}>
      <option value="en" lang="en">English</option>
      <option value="zh-TW" lang="zh-TW">繁體中文</option>
      <option value="ja" lang="ja">日本語</option>
    </select>
  </span>;
}
