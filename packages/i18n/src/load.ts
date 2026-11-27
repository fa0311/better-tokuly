import type { Language } from "./language";
import type { Namespace } from "./namespaces";

export { NAMESPACES, type Namespace } from "./namespaces";

/**
 * The shared namespaces one at a time, for an app that loads only the
 * language it shows and only the areas it opens (the web). Bundlers make
 * one chunk per file of this import. Metro cannot bundle it, so nothing the
 * phone app imports may reach this module.
 */
export async function loadNamespace(
  language: Language,
  namespace: Namespace,
): Promise<Record<string, unknown>> {
  return (await import(`./${language}/${namespace}.ts`)).default;
}
