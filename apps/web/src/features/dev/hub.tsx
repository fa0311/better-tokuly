import { Code } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the developer platform from the hub. */
export default [
  {
    id: "dev",
    section: "me",
    order: 60,
    to: "/developers",
    label: "dev:tile",
    icon: Code,
    tint: "bg-sky",
    keywords: [
      "developers",
      "api",
      "token",
      "oauth",
      "bot",
      "webhook",
      "開発",
      "デベロッパー",
      "トークン",
      "ボット",
    ],
  },
] satisfies HubTile[];
