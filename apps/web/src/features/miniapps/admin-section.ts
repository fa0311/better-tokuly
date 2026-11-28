import { Sparkles } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The mini-app review queue in the staff console. */
export default {
  id: "mini-apps",
  order: 115,
  to: "/admin/mini-apps",
  icon: Sparkles,
  needs: "moderator",
  label: "miniapps:review.nav",
} satisfies AdminSection;
