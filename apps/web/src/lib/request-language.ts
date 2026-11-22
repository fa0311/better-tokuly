import { createServerFn } from "@tanstack/react-start";
import { getCookie, getRequestHeader } from "@tanstack/react-start/server";
import { LANGUAGE_COOKIE, type Language, normalizeLanguage } from "#/lib/i18n";

/**
 * The language to render a request in: the user's saved choice, else the
 * browser's first preferred language.
 */
export const getRequestLanguage = createServerFn({ method: "GET" }).handler(
  async (): Promise<Language> => {
    const saved = getCookie(LANGUAGE_COOKIE);
    if (saved === "ja" || saved === "en") return saved;
    const preferred = getRequestHeader("accept-language")?.split(",")[0]?.trim();
    return normalizeLanguage(preferred);
  },
);
