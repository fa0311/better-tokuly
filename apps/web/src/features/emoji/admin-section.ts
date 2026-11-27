import { SmilePlus } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "emoji",
  order: 110,
  to: "/admin/emoji",
  icon: SmilePlus,
  needs: "moderator",
  label: "admin:nav.emoji",
} satisfies AdminSection;
