import type { ResourceLanguage } from "i18next";
import { namespaces } from "./namespaces";

/** Japanese resources: one namespace per file in `./ja`. */
export default namespaces(
  import.meta.glob<{ default: ResourceLanguage[string] }>("./ja/*.ts", { eager: true }),
);
