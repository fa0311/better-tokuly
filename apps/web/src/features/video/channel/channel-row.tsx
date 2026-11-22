import { Link } from "@tanstack/react-router";
import type { common, video } from "@tokuly/api";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { formatCount } from "../format";
import { SubscribeButton } from "../subscribe-button";
import { ChannelAvatar, ChannelName } from "../video-card";

/** A channel in a list: face, name, subscribers, and the subscribe button (or `trailing` in its place). */
export function ChannelRow({
  channel,
  subscribed,
  notificationLevel,
  trailing,
}: {
  channel: common.ChannelSummary;
  subscribed: boolean;
  notificationLevel?: video.NotificationLevel;
  trailing?: ReactNode;
}) {
  const { t } = useTranslation("video");
  return (
    <div data-testid="channel-row" className="flex items-center gap-3 rounded-2xl px-3 py-2.5">
      <Link
        to="/c/$handle"
        params={{ handle: channel.handle }}
        className="shrink-0"
        aria-label={channel.name}
      >
        <ChannelAvatar channel={channel} size="md" outlined />
      </Link>
      <Link to="/c/$handle" params={{ handle: channel.handle }} className="min-w-0 flex-1">
        <ChannelName channel={channel} className="max-w-full font-bold" />
        <span className="block truncate text-sm text-ink-soft">
          @{channel.handle}・
          {t("channel.subscribers", {
            count: Number(channel.subscriberCount),
            formatted: formatCount(channel.subscriberCount),
          })}
        </span>
      </Link>
      {trailing ?? (
        <SubscribeButton
          channel={channel}
          subscribed={subscribed}
          notificationLevel={notificationLevel}
          size="sm"
        />
      )}
    </div>
  );
}
