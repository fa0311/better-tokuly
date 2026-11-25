import { en as shared } from "@tokuly/i18n";
import type { ResourceLanguage } from "i18next";
import { namespaces } from "./namespaces";

/**
 * English resources: the namespaces every app shares (`@tokuly/i18n`) and one
 * namespace per file in `./en` for the screens only the web app has.
 */
export default namespaces(
  import.meta.glob<{ default: ResourceLanguage[string] }>("./en/*.ts", { eager: true }),
  shared,
);
