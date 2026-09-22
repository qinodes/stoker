import { en } from "./en.ts";
import { messages as zhTW } from "./zh-TW.ts";
import { messages as ja } from "./ja.ts";

export type Locale = "en" | "zh-TW" | "ja";
export type TranslationKey = keyof typeof en;
export type Dictionary = Record<TranslationKey, string>;
type Placeholders<S extends string> = S extends `${string}{${infer P}}${infer Rest}` ? P | Placeholders<Rest> : never;
type ParametersFor<K extends TranslationKey> = Record<Placeholders<(typeof en)[K]>, string | number>;
type TranslationArguments<K extends TranslationKey> = [Placeholders<(typeof en)[K]>] extends [never] ? [] : [parameters: ParametersFor<K>];
export type Translator = <K extends TranslationKey>(key: K, ...args: TranslationArguments<K>) => string;
export type StaticKey = { [K in TranslationKey]: [Placeholders<(typeof en)[K]>] extends [never] ? K : never }[TranslationKey];
// Raw strings are reserved for data and messages supplied by the server.
export type UiMessage = string | { key: StaticKey };
export const uiMessage = (key: StaticKey): UiMessage => ({ key });
export const dictionaries: Record<Locale, Dictionary> = { en, "zh-TW": zhTW, ja };
export const LANGUAGE_STORAGE_KEY = "stoker.ui.language";

export function isLocale(value: unknown): value is Locale {
  return value === "en" || value === "zh-TW" || value === "ja";
}

export function resolveLocale(saved: unknown, languages: readonly string[]): Locale {
  if (isLocale(saved)) return saved;
  for (const language of languages) {
    const tag = language.toLowerCase().replaceAll("_", "-");
    if (tag === "ja" || tag.startsWith("ja-")) return "ja";
    if (tag === "en" || tag.startsWith("en-")) return "en";
    const parts = tag.split("-");
    if (parts[0] === "zh" && !parts.includes("hans") && (parts.includes("hant") || parts.includes("tw") || parts.includes("hk") || parts.includes("mo"))) return "zh-TW";
  }
  return "en";
}

export function translate<K extends TranslationKey>(locale: Locale, key: K, ...args: TranslationArguments<K>): string {
  const template = dictionaries[locale][key] || en[key];
  const parameters = args[0] as Record<string, string | number> | undefined;
  return template.replace(/\{([^}]+)\}/g, (placeholder, name: string) => String(parameters?.[name] ?? placeholder));
}

export function translateMessage(locale: Locale, message: UiMessage): string {
  return typeof message === "string" ? message : translate(locale, message.key);
}

const stateKeys: Record<string, StaticKey> = {
  DRAFT: "state.DRAFT", QUEUED: "state.QUEUED", STARTING: "state.STARTING", RUNNING: "state.RUNNING",
  CANCELLING: "state.CANCELLING", RECOVERING: "state.RECOVERING", SUCCEEDED: "state.SUCCEEDED", FAILED: "state.FAILED", CANCELLED: "state.CANCELLED", LOST: "state.LOST", UNKNOWN: "state.UNKNOWN",
};

export function translateState(locale: Locale, value?: string | null): string {
  const state = value || "UNKNOWN";
  const key = Object.hasOwn(stateKeys, state) ? stateKeys[state] : undefined;
  return key ? translate(locale, key) : state;
}
