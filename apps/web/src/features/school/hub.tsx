import { GraduationCap } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";
import { SchoolHubTile } from "./now-card";

/** The school tile: on the hub page it is a card that says the class that is on or next. */
export default [
  {
    id: "school",
    section: "play",
    order: 40,
    to: "/school",
    label: () => i18n.t("school:title"),
    icon: GraduationCap,
    tint: "bg-lime",
    keywords: ["school", "timetable", "study", "時間割", "勉強", "テスト", "学校"],
    Card: SchoolHubTile,
  },
] satisfies HubTile[];
