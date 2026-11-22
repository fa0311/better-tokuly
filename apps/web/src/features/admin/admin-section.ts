import { Flag, Gavel, LayoutDashboard, Megaphone, ScrollText, Users } from "lucide-react";
import i18n from "#/lib/i18n";
import type { AdminSection } from "#/seams/admin";

const label = (key: string) => () => i18n.t(`admin:nav.${key}`);

/** The console's own sections; the review pages of other domains follow (their `admin-section.ts`). */
export default [
  {
    id: "dashboard",
    order: 0,
    to: "/admin",
    icon: LayoutDashboard,
    needs: "staff",
    label: label("dashboard"),
  },
  {
    id: "moderation",
    order: 10,
    to: "/admin/moderation",
    icon: Gavel,
    needs: "moderator",
    label: label("moderation"),
  },
  {
    id: "users",
    order: 20,
    to: "/admin/users",
    icon: Users,
    needs: "moderator",
    label: label("users"),
  },
  { id: "flags", order: 30, to: "/admin/flags", icon: Flag, needs: "admin", label: label("flags") },
  {
    id: "announcements",
    order: 40,
    to: "/admin/announcements",
    icon: Megaphone,
    needs: "admin",
    label: label("announcements"),
  },
  {
    id: "audit",
    order: 50,
    to: "/admin/audit",
    icon: ScrollText,
    needs: "admin",
    label: label("audit"),
  },
] satisfies AdminSection[];
