import { Mic } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the audio rooms and hangouts from the hub. */
export default [
  {
    id: "spaces",
    section: "play",
    order: 40,
    to: "/spaces",
    label: "spaces:tile",
    icon: Mic,
    tint: "bg-sky",
    badge: "spaces",
    keywords: ["spaces", "room", "voice", "audio", "hangout", "ルーム", "たまり場", "通話", "音声"],
  },
] satisfies HubTile[];
