import { type Language, normalizeLanguage } from "@tokuly/i18n/language";
import {
  loadNamespace as loadShared,
  type Namespace,
  NAMESPACES as SHARED,
} from "@tokuly/i18n/load";
import { type BackendModule, createInstance, type i18n as I18n, type Resource } from "i18next";
import { initReactI18next } from "react-i18next";

export { type Language, normalizeLanguage, SUPPORTED_LANGUAGES } from "@tokuly/i18n/language";

/** Cookie remembering the chosen language, so the server renders in it too. */
export const LANGUAGE_COOKIE = "tokuly_lang";

/**
 * The strings, one chunk per namespace and language: the shared namespaces
 * (`@tokuly/i18n`) and one per file in `src/i18n/<language>/` for the
 * screens only the web app has, named after the file. The browser loads the
 * language it shows, and of it only what the page in front of it uses.
 */
const OWN = import.meta.glob<{ default: Record<string, unknown> }>("../i18n/*/*.ts");

function ownFile(language: string, namespace: string): string {
  return `../i18n/${language}/${namespace}.ts`;
}

async function loadNamespace(language: Language, namespace: string) {
  const own = OWN[ownFile(language, namespace)];
  if (own) return (await own()).default;
  if ((SHARED as readonly string[]).includes(namespace)) {
    return loadShared(language, namespace as Namespace);
  }
  throw new Error(`no strings named "${namespace}"`);
}

/** What every page shows: the app's own words and the error texts. */
const ALWAYS = ["common", "errors"];

/** Namespaces the browser is fetching right now. */
let reading = 0;

const backend: BackendModule = {
  type: "backend",
  init() {},
  read(language, namespace, done) {
    reading++;
    loadNamespace(normalizeLanguage(language), namespace)
      .then(
        (strings) => done(null, strings),
        (error: Error) => done(error, false),
      )
      .finally(() => reading--);
  },
};

/**
 * The server renders any page in either language, so it holds every string:
 * the root route waits for `loadAllStrings()` before the first render.
 */
let everything: Resource | undefined;
let loadingEverything: Promise<void> | undefined;
export function loadAllStrings(): Promise<void> {
  loadingEverything ??= (async () => {
    const folder = "../i18n/ja/";
    const own = Object.keys(OWN)
      .filter((path) => path.startsWith(folder))
      .map((path) => path.slice(folder.length, -".ts".length));
    const names = [...SHARED, ...own].sort();
    const resources: Resource = {};
    for (const language of ["ja", "en"] as const) {
      const loaded = await Promise.all(names.map((name) => loadNamespace(language, name)));
      resources[language] = Object.fromEntries(names.map((name, i) => [name, loaded[i] ?? {}]));
    }
    everything = resources;
    for (const [language, namespaces] of Object.entries(resources)) {
      for (const [name, strings] of Object.entries(namespaces)) {
        i18n.addResourceBundle(language, name, strings);
      }
    }
  })();
  return loadingEverything;
}

const OPTIONS = {
  fallbackLng: "ja",
  defaultNS: "common",
  interpolation: { escapeValue: false },
  returnNull: false,
} as const;

/** The server's instance for one request (after `loadAllStrings()`). */
export function createI18n(language: Language): I18n {
  if (!everything) throw new Error("loadAllStrings() has not finished");
  return createServerI18n(language, everything);
}

function createServerI18n(language: Language, resources: Resource): I18n {
  const instance = createInstance();
  void instance
    .use(initReactI18next)
    .init({ ...OPTIONS, resources, lng: language, initAsync: false });
  return instance;
}

/**
 * The browser's instance. It starts in the language the server rendered
 * (`<html lang>`), so hydration always matches. A component's namespaces
 * load when `useTranslation(namespace)` first asks; until the page has
 * hydrated it suspends for them, so the first render says what the server
 * said (`stopSuspendingForStrings`). A lookup into a namespace nobody has
 * loaded yet (`t("chat:…")` from another feature, `i18n.t` outside a
 * component) shows nothing, starts loading it, and every `useTranslation`
 * renders again once it is there.
 */
function createBrowserI18n(language: Language): I18n {
  const instance = createInstance();
  void instance
    .use(backend)
    .use(initReactI18next)
    .init({
      ...OPTIONS,
      // i18next fetches every fallback language's chunk next to the shown one; English has
      // every Japanese key (`src/i18n/strings.test.ts`), so the browser needs only its own.
      fallbackLng: false,
      lng: language,
      ns: ALWAYS,
      partialBundledLanguages: true,
      react: { bindI18n: "languageChanged loaded" },
      appendNamespaceToMissingKey: true,
      parseMissingKeyHandler(missing, defaultValue) {
        const separator = missing.indexOf(":");
        const namespace = missing.slice(0, separator);
        if (!instance.hasLoadedNamespace(namespace)) {
          void instance.loadNamespaces(namespace);
          return "";
        }
        return defaultValue ?? missing.slice(separator + 1);
      },
    });
  return instance;
}

/**
 * The instance code outside React uses (`i18n.t`): the browser's, or on the
 * server one in Japanese, filled by `loadAllStrings()`.
 */
const i18n =
  typeof document === "undefined"
    ? createServerI18n("ja", {})
    : createBrowserI18n(normalizeLanguage(document.documentElement.lang));

/**
 * The namespaces of some `namespace:key` keys, for `useTranslation(…)` to
 * load before a list of other features' names (the hub, the settings) shows.
 */
export function namespacesOf(keys: readonly string[]): string[] {
  return [...new Set(keys.map((key) => key.slice(0, key.indexOf(":"))))];
}

/**
 * After hydration a namespace that is still loading shows nothing for the
 * moment it takes instead of suspending: a suspended update would take the
 * screen away under someone's hands (a composer that just opened its stamp
 * button lost what was being typed).
 *
 * Called from the root's first effect, but the page is not hydrated yet
 * then: a screen that suspended for its strings hydrates when they arrive,
 * and only then asks for its next namespace. Turning suspense off at once
 * rendered those screens without their words, against the server's HTML
 * (`/login` lost its title and React threw the page away). So it waits
 * until no strings have been loading for a few idle moments.
 */
export function stopSuspendingForStrings(): void {
  const later = (next: () => void) =>
    typeof requestIdleCallback === "function"
      ? requestIdleCallback(next, { timeout: 200 })
      : setTimeout(next, 50);
  let quiet = 0;
  const settle = () => {
    quiet = reading === 0 ? quiet + 1 : 0;
    if (quiet < 3) {
      later(settle);
      return;
    }
    i18n.options.react = { ...i18n.options.react, useSuspense: false };
  };
  later(settle);
}

export function setLanguage(language: Language): void {
  // biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is not in every browser the app supports.
  document.cookie = `${LANGUAGE_COOKIE}=${language}; Path=/; Max-Age=31536000; SameSite=Lax`;
  document.documentElement.lang = language;
  void i18n.changeLanguage(language);
}

export default i18n;
