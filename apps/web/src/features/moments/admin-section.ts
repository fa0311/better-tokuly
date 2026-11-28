import { Zap } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's page in the staff console. */
export default {
  id: "moments",
  order: 120,
  to: "/admin/moments",
  icon: Zap,
  needs: "admin",
  label: "admin:nav.moments",
} satisfies AdminSection;
