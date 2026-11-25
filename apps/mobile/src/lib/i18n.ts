import { en, ja, normalizeLanguage } from "@tokuly/i18n";
import { getLocales } from "expo-localization";
import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";

/**
 * The app's i18next instance: the shared namespaces of `@tokuly/i18n`
 * (session, map, friends, profile, the mobile screens…), Japanese first, in
 * the device's language.
 */
const i18n = createInstance();
void i18n.use(initReactI18next).init({
  resources: { ja, en },
  lng: normalizeLanguage(getLocales()[0]?.languageTag),
  fallbackLng: "ja",
  defaultNS: "common",
  interpolation: { escapeValue: false },
  returnNull: false,
  initAsync: false,
});

export default i18n;
