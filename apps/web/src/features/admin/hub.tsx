import { ShieldCheck } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the staff console from the hub; only staff see it. */
export default [
  {
    id: "admin",
    section: "me",
    order: 90,
    to: "/admin",
    label: "common:nav.admin",
    icon: ShieldCheck,
    tint: "bg-coral",
    staffOnly: true,
    keywords: ["admin", "管理"],
  },
] satisfies HubTile[];
