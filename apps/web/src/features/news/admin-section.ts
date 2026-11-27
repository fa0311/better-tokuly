import { Newspaper } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "news",
  order: 100,
  to: "/admin/news",
  icon: Newspaper,
  needs: "moderator",
  label: "admin:nav.news",
} satisfies AdminSection;
