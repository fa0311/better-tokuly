import { createInstance, type i18n as I18n } from "i18next";
import { initReactI18next } from "react-i18next";
import en from "#/i18n/en";
import ja from "#/i18n/ja";

export const SUPPORTED_LANGUAGES = ["ja", "en"] as const;
export type Language = (typeof SUPPORTED_LANGUAGES)[number];

/** Cookie remembering the chosen language, so the server renders in it too. */
export const LANGUAGE_COOKIE = "tokuly_lang";

/** Japanese first: Tokuly's home audience. Anything else falls back to English. */
export function normalizeLanguage(input: string | null | undefined): Language {
  if (!input) return "ja";
  return input.toLowerCase().startsWith("ja") ? "ja" : "en";
}

/** A ready-to-use (synchronously initialized) i18n instance in one language. */
export function createI18n(language: Language): I18n {
  const instance = createInstance();
  void instance.use(initReactI18next).init({
    resources: { ja, en },
    lng: language,
    fallbackLng: "ja",
    defaultNS: "common",
    interpolation: { escapeValue: false },
    returnNull: false,
    initAsync: false,
  });
  return instance;
}

/**
 * The browser's instance. It starts in the language the server rendered
 * (`<html lang>`), so hydration always matches. On the server each request
 * gets its own instance instead (see the root route).
 */
const i18n = createI18n(
  typeof document === "undefined" ? "ja" : normalizeLanguage(document.documentElement.lang),
);

export function setLanguage(language: Language): void {
  // biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is not in every browser the app supports.
  document.cookie = `${LANGUAGE_COOKIE}=${language}; Path=/; Max-Age=31536000; SameSite=Lax`;
  document.documentElement.lang = language;
  void i18n.changeLanguage(language);
}

export default i18n;
