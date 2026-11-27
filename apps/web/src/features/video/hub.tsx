import { Clapperboard, History, ListVideo } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The video tiles of the hub: the studio, playlists and the watch history. */
export default [
  {
    id: "video-studio",
    section: "make",
    order: 30,
    to: "/studio",
    label: "video:meTiles.studio",
    icon: Clapperboard,
    tint: "bg-sun",
    keywords: ["studio", "upload", "channel", "スタジオ", "投稿", "チャンネル"],
  },
  {
    id: "video-playlists",
    section: "make",
    order: 40,
    to: "/watch/playlists",
    label: "video:meTiles.playlists",
    icon: ListVideo,
    tint: "bg-pink",
    keywords: ["playlists", "watch later", "再生リスト", "あとで見る"],
  },
  {
    id: "video-history",
    section: "make",
    order: 50,
    to: "/watch/history",
    label: "video:meTiles.history",
    icon: History,
    tint: "bg-sky",
    keywords: ["watch history", "視聴履歴"],
  },
] satisfies HubTile[];
