import { MailQuestionMark } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the question box from the hub. */
export default [
  {
    id: "ask",
    section: "play",
    order: 30,
    to: "/ask",
    label: "ask:tile",
    icon: MailQuestionMark,
    tint: "bg-sun",
    badge: "ask",
    keywords: ["ask", "question", "質問箱", "しつもん"],
  },
] satisfies HubTile[];
