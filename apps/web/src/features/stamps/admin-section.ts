import { Sticker } from "lucide-react";
import i18n from "#/lib/i18n";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "stamps",
  order: 70,
  to: "/admin/stamps",
  icon: Sticker,
  needs: "staff",
  label: () => i18n.t("admin:nav.stamps"),
} satisfies AdminSection;
