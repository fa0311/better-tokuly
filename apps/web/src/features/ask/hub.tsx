import { MailQuestionMark } from "lucide-react";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";

/** The way in to the question box from the hub. */
export default [
  {
    id: "ask",
    section: "play",
    order: 30,
    to: "/ask",
    label: () => i18n.t("ask:tile"),
    icon: MailQuestionMark,
    tint: "bg-sun",
    badge: "ask",
    keywords: ["ask", "question", "質問箱", "しつもん"],
  },
] satisfies HubTile[];
