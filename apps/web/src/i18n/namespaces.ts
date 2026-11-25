import type { ResourceLanguage } from "i18next";

/**
 * Turns the modules of one language folder into i18next resources: every file
 * is a namespace named after the file (`ja/friends.ts` → `friends`), so adding
 * a feature's strings needs no registration anywhere. The shared namespaces
 * join them, and all are kept in name order (error texts are looked up
 * through the namespaces in that order).
 */
export function namespaces(
  modules: Record<string, { default: ResourceLanguage[string] }>,
  shared: ResourceLanguage,
): ResourceLanguage {
  const own = Object.entries(modules).map(([path, module]) => [
    path.slice(path.lastIndexOf("/") + 1, -".ts".length),
    module.default,
  ]);
  return Object.fromEntries(
    [...Object.entries(shared), ...own].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}
