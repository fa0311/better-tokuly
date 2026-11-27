import { Sticker } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "stamps",
  order: 70,
  to: "/admin/stamps",
  icon: Sticker,
  needs: "staff",
  label: "admin:nav.stamps",
} satisfies AdminSection;
