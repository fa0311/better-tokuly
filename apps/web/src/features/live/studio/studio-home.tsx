import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { live, toMillis } from "@tokuly/api";
import { BarChart3, Gift, Layers, Plus, Scissors, ShieldCheck } from "lucide-react";
import type { ReactNode } from "react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { analyticsClient, liveKeys, nextCursor, streamClient } from "../api";
import { ClipGrid } from "../clips/clip-list";
import { visibilityOf } from "../labels";
import { Countdown, formatCount, formatMoment, StreamPicture } from "../parts";
import { clock } from "../player/live-player";
import { LiveChannelPicker, NoChannel, StatusChip, useLiveChannel } from "./shared";

const { StreamStatus } = live;

function StudioLink({ to, icon, label }: { to: string; icon: ReactNode; label: string }) {
  return (
    <Link
      to={to}
      className="pressable sticker-sm flex h-11 shrink-0 items-center gap-2 rounded-full bg-surface px-4 text-sm font-black [&>svg]:size-4"
    >
      {icon}
      {label}
    </Link>
  );
}

/** A stream of the caller as a row of the studio: where it stands and the way into its control room. */
function StudioStreamRow({ stream }: { stream: live.StreamSummary }) {
  const { t, i18n } = useTranslation("live");
  const visibility = visibilityOf(stream.visibility);
  const VisibilityIcon = visibility.icon;
  return (
    <li>
      <Link
        to="/studio/live/$streamId"
        params={{ streamId: stream.shortId }}
        className="flex items-start gap-3 rounded-2xl p-2 hover:bg-surface-2"
        data-testid="studio-stream"
      >
        <StreamPicture stream={stream} className="w-32 shrink-0" />
        <span className="min-w-0 flex-1">
          <span className="line-clamp-2 leading-snug font-black">
            <EmojiText text={stream.title} />
          </span>
          <span className="mt-1 flex flex-wrap items-center gap-1.5">
            <StatusChip status={stream.status} />
            <span className="inline-flex h-6 items-center gap-1 rounded-full bg-surface-2 px-2.5 text-xs font-black text-ink-soft">
              <VisibilityIcon className="size-3.5" />
              {t(`visibility.${visibility.key}`)}
            </span>
          </span>
          <span className="mt-1 block text-xs font-bold text-ink-faint">
            {stream.status === StreamStatus.UPCOMING && stream.scheduledAt ? (
              <>
                {formatMoment(toMillis(stream.scheduledAt), i18n.language)}
                {" · "}
                <Countdown targetMs={toMillis(stream.scheduledAt)} />
              </>
            ) : stream.status === StreamStatus.LIVE ? (
              t("viewers", { formatted: formatCount(stream.viewerCount, i18n.language) })
            ) : stream.endedAt ? (
              formatMoment(toMillis(stream.endedAt), i18n.language)
            ) : (
              t("status.waiting")
            )}
          </span>
        </span>
      </Link>
    </li>
  );
}

function ChannelStreams({ channelId }: { channelId: string }) {
  const { t } = useTranslation("live");
  const query = useInfiniteQuery({
    queryKey: liveKeys.channelStreams(channelId, StreamStatus.UNSPECIFIED),
    queryFn: ({ pageParam }) =>
      streamClient().listChannelStreams({ channelId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const streams = query.data?.pages.flatMap((page) => page.streams) ?? [];
  if (query.isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-20 w-full" />
      </div>
    );
  if (streams.length === 0)
    return <EmptyState emoji="🎙️" title={t("studio.emptyTitle")} body={t("studio.emptyBody")} />;
  const open = streams.filter((stream) => stream.status !== StreamStatus.ENDED);
  const past = streams.filter((stream) => stream.status === StreamStatus.ENDED);
  return (
    <>
      {open.length > 0 && (
        <section data-testid="studio-open-streams">
          <h2 className="mb-1 text-lg font-black">{t("studio.open")}</h2>
          <ul className="flex flex-col">
            {open.map((stream) => (
              <StudioStreamRow key={stream.id} stream={stream} />
            ))}
          </ul>
        </section>
      )}
      {past.length > 0 && (
        <section className="mt-5">
          <h2 className="mb-1 text-lg font-black">{t("studio.past")}</h2>
          <ul className="flex flex-col">
            {past.map((stream) => (
              <StudioStreamRow key={stream.id} stream={stream} />
            ))}
          </ul>
        </section>
      )}
      <LoadMore query={query} />
    </>
  );
}

/** The streams of a channel that went on air, with their headline numbers. */
function Reports({ channelId }: { channelId: string }) {
  const { t, i18n } = useTranslation("live");
  const query = useInfiniteQuery({
    queryKey: liveKeys.reports(channelId),
    queryFn: ({ pageParam }) =>
      analyticsClient().listStreamReports({ channelId, page: { cursor: pageParam, limit: 20 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const reports = query.data?.pages.flatMap((page) => page.reports) ?? [];
  if (reports.length === 0) return null;
  const count = (value: bigint) => formatCount(value, i18n.language);
  return (
    <section className="mt-6" data-testid="stream-reports">
      <h2 className="mb-2 flex items-center gap-2 text-lg font-black">
        <BarChart3 className="size-5" />
        {t("studio.reports")}
      </h2>
      <div className="overflow-x-auto rounded-2xl border-2 border-line">
        <table className="w-full min-w-[32rem] text-sm">
          <thead>
            <tr className="bg-surface-2 text-left text-xs font-black text-ink-soft">
              <th className="px-3 py-2">{t("studio.report.stream")}</th>
              <th className="px-3 py-2 text-right">{t("studio.report.duration")}</th>
              <th className="px-3 py-2 text-right">{t("studio.report.peak")}</th>
              <th className="px-3 py-2 text-right">{t("studio.report.chat")}</th>
              <th className="px-3 py-2 text-right">{t("studio.report.gifts")}</th>
            </tr>
          </thead>
          <tbody>
            {reports.map((report) => (
              <tr key={report.stream?.id} className="border-t-2 border-line">
                <td className="max-w-56 px-3 py-2">
                  <Link
                    to="/studio/live/$streamId/analytics"
                    params={{ streamId: report.stream?.shortId ?? "" }}
                    className="block truncate font-bold hover:underline"
                  >
                    <EmojiText text={report.stream?.title ?? ""} />
                  </Link>
                </td>
                <td className="px-3 py-2 text-right tabular-nums">
                  {clock(Number(report.durationSeconds))}
                </td>
                <td className="px-3 py-2 text-right tabular-nums">{count(report.peakViewers)}</td>
                <td className="px-3 py-2 text-right tabular-nums">
                  {count(report.chatMessageCount)}
                </td>
                <td className="px-3 py-2 text-right tabular-nums">{count(report.giftCoins)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <LoadMore query={query} />
    </section>
  );
}

/** `/studio/live`: the caller's streams, the way to start one, and the tools around streaming. */
export function LiveStudioHome() {
  const { t } = useTranslation("live");
  const me = useMe();
  const { query, channels, channel } = useLiveChannel();
  const channelId = channel?.summary?.id;
  return (
    <Page
      title={t("studio.title")}
      back="/studio"
      wide
      actions={
        channelId && (
          <Link
            to="/studio/live/new"
            className="pressable sticker-sm inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full bg-brand px-3.5 text-sm font-black text-on-brand"
          >
            <Plus className="size-4" />
            {t("studio.new")}
          </Link>
        )
      }
    >
      {query.isPending ? (
        <PaneSpinner />
      ) : !channelId ? (
        <NoChannel />
      ) : (
        <>
          <LiveChannelPicker channels={channels} selectedId={channelId} />
          <nav
            className="scrollbar-none -mx-4 mb-5 flex gap-2 overflow-x-auto px-4 pb-1"
            aria-label={t("studio.tools")}
          >
            <StudioLink to="/studio/live/overlays" icon={<Layers />} label={t("overlays.title")} />
            <StudioLink
              to="/studio/live/moderation"
              icon={<ShieldCheck />}
              label={t("moderation.title")}
            />
            <StudioLink to="/live/gifts" icon={<Gift />} label={t("giftHistory.title")} />
          </nav>
          <ChannelStreams key={channelId} channelId={channelId} />
          <Reports key={`reports-${channelId}`} channelId={channelId} />
          {me?.user && (
            <section className="mt-6">
              <h2 className="mb-2 flex items-center gap-2 text-lg font-black">
                <Scissors className="size-5" />
                {t("clip.mine")}
              </h2>
              <ClipGrid kind="creator" id={me.user.id} mine />
            </section>
          )}
        </>
      )}
    </Page>
  );
}
