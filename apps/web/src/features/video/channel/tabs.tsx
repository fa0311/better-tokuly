import type { video } from "@tokuly/api";
import type { ComponentType } from "react";
import i18n from "#/lib/i18n";
import { ChannelAbout } from "./channel-about";
import { ChannelPlaylists } from "./channel-playlists";
import { ChannelSubscribers } from "./channel-subscribers";
import { ChannelVideos } from "./channel-videos";

/**
 * A tab of the channel page, shown at `/c/<handle>/<id>`.
 *
 * Other features add theirs without touching this folder: default-export a
 * `ChannelTab` from `src/features/<area>/channel-tab.tsx` and it appears on
 * every channel page, sorted by `order` (videos 10, playlists 40, about 90 —
 * shorts and live fit between videos and playlists).
 *
 *   export default {
 *     id: "shorts",
 *     order: 20,
 *     label: () => i18n.t("shorts:channelTab"),
 *     Component: ({ channel }) => <ChannelShorts channelId={channel.summary?.id ?? ""} />,
 *   } satisfies ChannelTab;
 */
export interface ChannelTab {
  /** The path segment after the handle; letters, digits and dashes. */
  id: string;
  order: number;
  /** The tab's name in the current language. */
  label(): string;
  /** Only the channel's owner sees the tab. */
  ownerOnly?: boolean;
  Component: ComponentType<{ channel: video.Channel }>;
}

const label = (key: string) => () => i18n.t(`channel.tabs.${key}`, { ns: "video" });

const OWN: ChannelTab[] = [
  { id: "videos", order: 10, label: label("videos"), Component: ChannelVideos },
  { id: "playlists", order: 40, label: label("playlists"), Component: ChannelPlaylists },
  { id: "about", order: 90, label: label("about"), Component: ChannelAbout },
  { id: "subscribers", order: 100, label: label("subscribers"), ownerOnly: true, Component: ChannelSubscribers },
];

const contributed = import.meta.glob<{ default: ChannelTab }>("../../*/channel-tab.tsx", { eager: true });

/** Every tab after "home", in display order. */
export const CHANNEL_TABS: ChannelTab[] = [...OWN, ...Object.values(contributed).map((module) => module.default)].sort((a, b) => a.order - b.order);

export function channelTab(id: string): ChannelTab | undefined {
  return CHANNEL_TABS.find((tab) => tab.id === id);
}
