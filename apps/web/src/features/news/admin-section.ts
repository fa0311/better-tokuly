import { Newspaper } from "lucide-react";
import i18n from "#/lib/i18n";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "news",
  order: 100,
  to: "/admin/news",
  icon: Newspaper,
  needs: "moderator",
  label: () => i18n.t("admin:nav.news"),
} satisfies AdminSection;
