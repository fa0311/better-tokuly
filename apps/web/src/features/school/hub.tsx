import { GraduationCap } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The school tile: on the hub page it is a card that says the class that is on or next. */
export default [
  {
    id: "school",
    section: "play",
    order: 40,
    to: "/school",
    label: "school:title",
    icon: GraduationCap,
    tint: "bg-lime",
    keywords: ["school", "timetable", "study", "時間割", "勉強", "テスト", "学校"],
    card: () => import("./now-card").then((module) => ({ default: module.SchoolHubTile })),
  },
] satisfies HubTile[];
