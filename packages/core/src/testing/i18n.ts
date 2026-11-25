import { en, ja } from "@tokuly/i18n";
import { createInstance, type i18n as I18n, type Resource } from "i18next";

/**
 * An i18next instance over the shared strings, for tests: what an app builds
 * at start-up, minus its own screens. `extra` adds namespaces per language.
 */
export function testI18n(language: "ja" | "en" = "ja", extra: Resource = {}): I18n {
  const instance = createInstance();
  void instance.init({
    resources: {
      ja: { ...ja, ...extra.ja },
      en: { ...en, ...extra.en },
    },
    lng: language,
    fallbackLng: "ja",
    defaultNS: "common",
    interpolation: { escapeValue: false },
    returnNull: false,
    initAsync: false,
  });
  return instance;
}
