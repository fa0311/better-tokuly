import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "watch/index", options: () => ({ title: i18n.t("video:feeds.title") }) },
  // The video's own title is under the player; the header stays short.
  { name: "watch/[videoId]", options: () => ({ title: "" }) },
  { name: "watch/library", options: () => ({ title: i18n.t("video:app.library") }) },
  { name: "watch/history", options: () => ({ title: i18n.t("video:history.title") }) },
  { name: "watch/channels", options: () => ({ title: i18n.t("video:subscriptions.title") }) },
  // Channel and playlist screens set their name once it has loaded.
  { name: "c/[handle]", options: () => ({ title: "" }) },
  { name: "playlist/[id]", options: () => ({ title: "" }) },
] satisfies ScreenEntry[];
