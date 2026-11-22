import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { common, type video } from "@tokuly/api";
import { Clapperboard, Pencil } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { SubjectMoreButton } from "#/features/engagement";
import { channelClient, videoKeys } from "../api";
import { formatCount } from "../format";
import { SubscribeButton } from "../subscribe-button";
import { ChannelAvatar, ChannelName } from "../video-card";
import { CHANNEL_TABS } from "./tabs";

/** The channel page's data: header and home sections in one read, shared by the layout and every tab. */
export function useChannelPage(handle: string) {
  return useQuery({
    queryKey: videoKeys.channel(handle),
    queryFn: () => channelClient().getChannelPage({ channelId: handle }),
  });
}

/**
 * Frame of `/c/<handle>`: banner, face, name, numbers, subscribe — and the
 * tabs, each a route below it.
 */
export function ChannelLayout({ handle }: { handle: string }) {
  const { t } = useTranslation("video");
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const page = useChannelPage(handle);
  const channel = page.data?.channel;
  const summary = channel?.summary;
  // The tabs can outgrow a phone (more so once shorts and live join them): the open one is brought into view.
  const tabBar = useRef<HTMLDivElement>(null);
  // re-run whenever another tab opens.
  useEffect(() => {
    tabBar.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [pathname]);

  if (page.isPending) {
    return (
      <div className="h-full overflow-y-auto" aria-busy>
        <Skeleton className="h-32 w-full rounded-none md:h-48" />
        <div className="mx-auto max-w-6xl px-4 pt-4">
          <Skeleton className="h-24" />
          <Skeleton className="mt-4 h-10 w-80 max-w-full" />
        </div>
      </div>
    );
  }
  if (page.isError || !channel || !summary) {
    return (
      <div className="grid h-full place-items-center">
        <EmptyState emoji="📡" title={t("channel.notFound")} body={t("channel.notFoundBody")} />
      </div>
    );
  }

  const isOwner = channel.viewer?.isOwner ?? false;
  const tabs = CHANNEL_TABS.filter((tab) => !tab.ownerOnly || isOwner);
  // `/c/<handle>/<tab>`: the segment after the handle names the open tab.
  const segment = decodeURIComponent(pathname.split("/")[3] ?? "");
  const active = tabs.find((tab) => tab.id === segment)?.id ?? "home";
  const banner = channel.banner?.url;

  return (
    <div className="h-full overflow-y-auto pb-32 md:pb-10" data-testid="channel-page">
      <div
        className="h-28 w-full bg-cover bg-center sm:h-40 lg:h-52"
        style={
          banner
            ? {
                backgroundImage: `url(${banner})`,
                backgroundColor: channel.banner?.dominantColor || undefined,
              }
            : {
                background: `linear-gradient(120deg, ${summary.icon?.dominantColor || "var(--brand)"}, var(--pink))`,
              }
        }
      />
      <div className="mx-auto max-w-6xl px-4">
        <header className="flex flex-col gap-4 pt-4 sm:flex-row sm:items-center">
          <ChannelAvatar
            channel={summary}
            size="xl"
            outlined
            className="-mt-12 self-start sm:mt-0"
          />
          <div className="min-w-0 flex-1">
            <h1 className="text-2xl font-black" data-testid="channel-name">
              <ChannelName channel={summary} className="max-w-full" />
            </h1>
            <p className="mt-0.5 flex flex-wrap gap-x-3 text-sm font-bold text-ink-soft">
              <span>@{summary.handle}</span>
              <span data-testid="subscriber-count">
                {t("channel.subscribers", {
                  count: Number(summary.subscriberCount),
                  formatted: formatCount(summary.subscriberCount),
                })}
              </span>
              <span>
                {t("channel.videoCount", {
                  count: Number(channel.videoCount),
                  formatted: formatCount(channel.videoCount),
                })}
              </span>
            </p>
            {channel.description && (
              <Link
                to="/c/$handle/$tab"
                params={{ handle: summary.handle, tab: "about" }}
                className="mt-1 block truncate text-sm text-ink-soft hover:text-ink"
              >
                {channel.description.split("\n")[0]}
              </Link>
            )}
          </div>
          <div className="flex shrink-0 flex-wrap gap-2">
            {isOwner ? (
              <>
                <Link
                  to="/studio/channels/$channelId"
                  params={{ channelId: summary.id }}
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
                >
                  <Pencil className="size-4" />
                  {t("channel.edit")}
                </Link>
                <Link
                  to="/studio"
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
                >
                  <Clapperboard className="size-4" />
                  {t("studio.title")}
                </Link>
              </>
            ) : (
              <SubscribeButton
                channel={summary}
                subscribed={channel.viewer?.subscribed ?? false}
                notificationLevel={channel.viewer?.notificationLevel}
              />
            )}
            <SubjectMoreButton
              subject={{ type: common.SubjectType.CHANNEL, id: summary.id }}
              title={summary.name}
              label={`${summary.name} (@${summary.handle})`}
              report={!isOwner}
            />
          </div>
        </header>

        <div ref={tabBar} className="mt-5">
          <Tabs
            value={active}
            onChange={(next) =>
              void (next === "home"
                ? navigate({ to: "/c/$handle", params: { handle: summary.handle } })
                : navigate({
                    to: "/c/$handle/$tab",
                    params: { handle: summary.handle, tab: next },
                  }))
            }
            items={[
              { value: "home", label: t("channel.tabs.home") },
              ...tabs.map((tab) => ({ value: tab.id, label: tab.label() })),
            ]}
          />
        </div>
        <div className="pt-5">
          <Outlet />
        </div>
      </div>
    </div>
  );
}

/** One of the tabs after "home": looked up by the path segment. */
export function ChannelTabPage({ handle, tab: tabId }: { handle: string; tab: string }) {
  const { t } = useTranslation("video");
  const page = useChannelPage(handle);
  const channel: video.Channel | undefined = page.data?.channel;
  if (!channel) return null;
  const tab = CHANNEL_TABS.find(
    (candidate) => candidate.id === tabId && (!candidate.ownerOnly || channel.viewer?.isOwner),
  );
  if (!tab) return <EmptyState emoji="🧭" title={t("channel.tabNotFound")} />;
  return <tab.Component channel={channel} />;
}
