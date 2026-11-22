import type { ResourceLanguage } from "i18next";
import { namespaces } from "./namespaces";

/** English resources: one namespace per file in `./en`. */
export default namespaces(
  import.meta.glob<{ default: ResourceLanguage[string] }>("./en/*.ts", { eager: true }),
);
