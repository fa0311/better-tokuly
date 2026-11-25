import { ja as shared } from "@tokuly/i18n";
import type { ResourceLanguage } from "i18next";
import { namespaces } from "./namespaces";

/**
 * Japanese resources: the namespaces every app shares (`@tokuly/i18n`) and one
 * namespace per file in `./ja` for the screens only the web app has.
 */
export default namespaces(
  import.meta.glob<{ default: ResourceLanguage[string] }>("./ja/*.ts", { eager: true }),
  shared,
);
