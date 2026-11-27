import { Search } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "search",
  order: 90,
  to: "/admin/search",
  icon: Search,
  needs: "admin",
  label: "admin:nav.search",
} satisfies AdminSection;
