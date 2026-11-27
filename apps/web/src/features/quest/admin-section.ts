import { Medal } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of the staff console that leads to giving and taking badges by hand. */
export default {
  id: "quest",
  order: 120,
  to: "/quest/admin",
  icon: Medal,
  needs: "admin",
  label: "quest:admin.nav",
} satisfies AdminSection;
