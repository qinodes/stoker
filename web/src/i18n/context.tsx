import { createContext, useContext, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { isLocale, LANGUAGE_STORAGE_KEY, resolveLocale, translate, translateMessage, translateState, type Locale, type Translator, type UiMessage } from "./messages.ts";
import { refreshLocalizedValidity } from "../form-validation.ts";

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
    refreshLocalizedValidity(locale);
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

const LANGUAGES = [
  { locale: "en", label: "English" },
  { locale: "zh-TW", label: "繁體中文" },
  { locale: "ja", label: "日本語" },
] as const;

export function LanguagePicker() {
  const { locale, setLocale, t } = useI18n();
  const [open, setOpen] = useState(false);
  const [focusedIndex, setFocusedIndex] = useState(0);
  const controlRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuId = useId();

  useEffect(() => {
    if (open) optionRefs.current[focusedIndex]?.focus();
  }, [open, focusedIndex]);

  useEffect(() => {
    if (!open) return;
    const dismissOutside = (event: Event) => {
      if (event.target instanceof Node && !controlRef.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("focusin", dismissOutside);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("focusin", dismissOutside);
    };
  }, [open]);

  const openMenu = (index: number) => { setFocusedIndex(index); setOpen(true); };
  const closeMenu = () => { setOpen(false); buttonRef.current?.focus(); };

  return <div className="language-control" ref={controlRef}>
    <button ref={buttonRef} className="language-picker" type="button" aria-label={t("language.label")} title={t("language.label")} aria-haspopup="menu" aria-expanded={open} aria-controls={open ? menuId : undefined} onClick={() => {
      if (open) setOpen(false);
      else openMenu(LANGUAGES.findIndex(language => language.locale === locale));
    }} onKeyDown={(event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        openMenu(event.key === "ArrowDown" ? 0 : LANGUAGES.length - 1);
      }
    }}>
      <svg className="language-globe" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true" focusable="false">
        <circle cx="12" cy="12" r="9" />
        <ellipse cx="12" cy="12" rx="4" ry="9" />
        <path d="M3 12h18" />
      </svg>
    </button>
    {open && <div className="language-menu" id={menuId} role="menu" aria-label={t("language.label")} onKeyDown={(event) => {
      switch (event.key) {
        case "ArrowDown": event.preventDefault(); setFocusedIndex((focusedIndex + 1) % LANGUAGES.length); break;
        case "ArrowUp": event.preventDefault(); setFocusedIndex((focusedIndex + LANGUAGES.length - 1) % LANGUAGES.length); break;
        case "Home": event.preventDefault(); setFocusedIndex(0); break;
        case "End": event.preventDefault(); setFocusedIndex(LANGUAGES.length - 1); break;
        case "Escape": event.preventDefault(); event.stopPropagation(); closeMenu(); break;
        case "Tab": closeMenu(); break;
        default: {
          if (event.key.length !== 1 || event.altKey || event.ctrlKey || event.metaKey) break;
          const match = LANGUAGES.findIndex(language => language.label.toLowerCase().startsWith(event.key.toLowerCase()));
          if (match >= 0) { event.preventDefault(); setFocusedIndex(match); }
        }
      }
    }}>
      {LANGUAGES.map((language, index) => <button key={language.locale} ref={(element) => { optionRefs.current[index] = element; }} className="language-option" type="button" role="menuitemradio" aria-checked={locale === language.locale} lang={language.locale} data-locale={language.locale} tabIndex={focusedIndex === index ? 0 : -1} onFocus={() => setFocusedIndex(index)} onClick={() => {
        setLocale(language.locale);
        closeMenu();
      }}>
        <span>{language.label}</span>
        <svg className="language-check" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true" focusable="false">
          {locale === language.locale && <path d="m3 8 3 3 7-7" />}
        </svg>
      </button>)}
    </div>}
  </div>;
}
