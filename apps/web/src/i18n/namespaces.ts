import type { ResourceLanguage } from "i18next";

/**
 * Turns the modules of one language folder into i18next resources: every file
 * is a namespace named after the file (`ja/friends.ts` → `friends`), so adding
 * a feature's strings needs no registration anywhere.
 */
export function namespaces(
  modules: Record<string, { default: ResourceLanguage[string] }>,
): ResourceLanguage {
  return Object.fromEntries(
    Object.entries(modules).map(([path, module]) => [
      path.slice(path.lastIndexOf("/") + 1, -".ts".length),
      module.default,
    ]),
  );
}
