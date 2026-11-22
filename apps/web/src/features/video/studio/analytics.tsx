import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, type video } from "@tokuly/api";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { analyticsClient, videoKeys } from "../api";
import { formatClock, formatCount, formatWatchTime, isoDay } from "../format";
import { TRAFFIC_SOURCE_KEYS } from "../labels";
import { Thumbnail } from "../video-card";
import {
  ChartCard,
  type ChartPoint,
  ColumnChart,
  RankedBars,
  StatTile,
  TrendChart,
} from "./charts";

export const RANGES = [7, 28, 90] as const;
export type RangeDays = (typeof RANGES)[number];

/** The last `days` calendar days up to today, in the viewer's own time zone. */
function dateRange(days: number): { startDate: string; endDate: string; timeZone: string } {
  const end = new Date();
  const start = new Date(end.getFullYear(), end.getMonth(), end.getDate() - (days - 1));
  return {
    startDate: isoDay(start),
    endDate: isoDay(end),
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}

/** The one filter of the analytics: which period every number and chart below it covers. */
export function RangePicker({
  days,
  onChange,
}: {
  days: RangeDays;
  onChange(days: RangeDays): void;
}) {
  const { t } = useTranslation("video");
  return (
    <Tabs<`${RangeDays}`>
      value={`${days}`}
      onChange={(value) => onChange(Number(value) as RangeDays)}
      items={RANGES.map((range) => ({
        value: `${range}` as const,
        label: t("studio.analytics.lastDays", { count: range }),
      }))}
    />
  );
}

type Metric = "views" | "watchTime" | "subscribers";

interface Report {
  totals?: video.AnalyticsTotals;
  daily: video.DailyAnalytics[];
  trafficSources: video.TrafficSourceStats[];
}

function AnalyticsUnavailable({ error }: { error: unknown }) {
  const { t } = useTranslation("video");
  return errorReason(error) === "ANALYTICS_UNAVAILABLE" ? (
    <EmptyState
      emoji="🛠️"
      title={t("studio.analytics.unavailable")}
      body={t("studio.analytics.unavailableBody")}
    />
  ) : (
    <EmptyState emoji="😵‍💫" title={errorMessage(error)} />
  );
}

function ReportSkeleton() {
  return (
    <div aria-busy>
      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        {[0, 1, 2, 3].map((tile) => (
          <Skeleton key={tile} className="h-20" />
        ))}
      </div>
      <Skeleton className="mt-4 h-64 rounded-card" />
    </div>
  );
}

/** Headline numbers, the daily trend and where viewers came from: the part channel and video analytics share. */
function ReportView({
  report,
  stale,
  subscriberCount,
  children,
}: {
  report: Report /** A newer period is loading: hold this one, dimmed. */;
  stale: boolean;
  subscriberCount?: bigint;
  children?: ReactNode;
}) {
  const { t, i18n } = useTranslation("video");
  const [metric, setMetric] = useState<Metric>("views");
  const totals = report.totals;
  const number = (value: number | bigint) => Number(value).toLocaleString(i18n.language);

  const day = (iso: string, long: boolean) =>
    new Date(`${iso}T00:00:00`).toLocaleDateString(
      i18n.language,
      long
        ? { month: "long", day: "numeric", weekday: "short" }
        : { month: "numeric", day: "numeric" },
    );
  const points: ChartPoint[] = report.daily.map((entry) => ({
    value:
      metric === "views"
        ? Number(entry.views)
        : metric === "watchTime"
          ? entry.watchTimeSeconds
          : Number(entry.subscribersGained),
    label: day(entry.date, false),
    longLabel: day(entry.date, true),
  }));
  const format = (value: number) =>
    metric === "watchTime"
      ? formatWatchTime(value)
      : value >= 10_000
        ? formatCount(value)
        : number(Math.round(value));
  const totalViews = report.trafficSources.reduce((sum, source) => sum + Number(source.views), 0);

  return (
    <div className={cn("flex flex-col gap-4 transition-opacity", stale && "opacity-50")}>
      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        <StatTile
          label={t("studio.analytics.views")}
          value={formatCount(totals?.views ?? 0n)}
          detail={t("studio.analytics.uniqueViewers", {
            formatted: formatCount(totals?.uniqueViewers ?? 0n),
          })}
        />
        <StatTile
          label={t("studio.analytics.watchTime")}
          value={formatWatchTime(totals?.watchTimeSeconds ?? 0)}
          detail={t("studio.analytics.averageView", {
            time: formatClock(totals?.averageViewDurationSeconds ?? 0),
          })}
        />
        <StatTile
          label={t("studio.analytics.subscribers")}
          value={`+${number(totals?.subscribersGained ?? 0n)}`}
          detail={
            subscriberCount === undefined
              ? t("studio.analytics.subscribersLost", {
                  formatted: number(totals?.subscribersLost ?? 0n),
                })
              : t("studio.analytics.subscribersNow", {
                  formatted: number(subscriberCount),
                  lost: number(totals?.subscribersLost ?? 0n),
                })
          }
        />
        <StatTile
          label={t("studio.analytics.likes")}
          value={formatCount(totals?.likes ?? 0n)}
          detail={t("studio.analytics.dislikes", {
            formatted: formatCount(totals?.dislikes ?? 0n),
          })}
        />
      </div>

      <ChartCard title={t(`studio.analytics.daily.${metric}`)}>
        <Tabs<Metric>
          className="mb-4"
          value={metric}
          onChange={setMetric}
          items={[
            { value: "views", label: t("studio.analytics.views") },
            { value: "watchTime", label: t("studio.analytics.watchTime") },
            { value: "subscribers", label: t("studio.analytics.newSubscribers") },
          ]}
        />
        <TrendChart
          title={t(`studio.analytics.daily.${metric}`)}
          points={points}
          format={format}
          emptyText={t("studio.analytics.noData")}
          positionHeader={t("studio.analytics.date")}
          keyboardLabel={t("studio.analytics.keyboard")}
        />
      </ChartCard>

      {children}

      <ChartCard title={t("studio.analytics.traffic")}>
        <RankedBars
          title={t("studio.analytics.traffic")}
          emptyText={t("studio.analytics.noData")}
          bars={report.trafficSources.map((source) => ({
            key: String(source.source),
            label: t(`traffic.${TRAFFIC_SOURCE_KEYS[source.source] ?? "other"}`),
            value: Number(source.views),
            valueText: t("studio.analytics.viewsShare", {
              formatted: number(source.views),
              percent: totalViews > 0 ? Math.round((Number(source.views) / totalViews) * 100) : 0,
            }),
          }))}
        />
      </ChartCard>
    </div>
  );
}

/** A channel's analytics over the chosen period: totals, trend, traffic, its most watched videos, and what is happening right now. */
export function ChannelAnalytics({ channelId, days }: { channelId: string; days: RangeDays }) {
  const { t, i18n } = useTranslation("video");
  const report = useQuery({
    queryKey: videoKeys.channelAnalytics(channelId, days),
    queryFn: () => analyticsClient().getChannelAnalytics({ channelId, range: dateRange(days) }),
    // While another period loads, the previous one stays on screen.
    placeholderData: (previous) => previous,
  });
  if (report.isPending) return <ReportSkeleton />;
  if (report.isError) return <AnalyticsUnavailable error={report.error} />;
  const data = report.data;

  return (
    <div className="flex flex-col gap-4">
      <RealtimeCard target={{ case: "channelId", value: channelId }} />
      <ReportView
        report={data}
        stale={report.isPlaceholderData}
        subscriberCount={data.subscriberCount}
      >
        <ChartCard title={t("studio.analytics.topVideos")}>
          {data.topVideos.length === 0 ? (
            <p className="py-6 text-center text-sm font-bold text-ink-faint">
              {t("studio.analytics.noData")}
            </p>
          ) : (
            <ol className="flex flex-col gap-1">
              {data.topVideos.map(
                (entry, index) =>
                  entry.video && (
                    <TopVideo
                      key={entry.video.id}
                      rank={index + 1}
                      entry={entry}
                      language={i18n.language}
                    />
                  ),
              )}
            </ol>
          )}
        </ChartCard>
      </ReportView>
    </div>
  );
}

function TopVideo({
  rank,
  entry,
  language,
}: {
  rank: number;
  entry: video.VideoStats;
  language: string;
}) {
  const { t } = useTranslation("video");
  const summary = entry.video;
  if (!summary) return null;
  return (
    <li>
      <Link
        to="/studio/videos/$videoId"
        params={{ videoId: summary.shortId }}
        search={{ tab: "analytics" }}
        className="flex items-center gap-3 rounded-2xl p-1.5 hover:bg-surface-2"
      >
        <span className="w-5 shrink-0 text-center text-sm font-black text-ink-faint tabular-nums">
          {rank}
        </span>
        <Thumbnail video={summary} sizes={96} className="w-24 shrink-0 rounded-lg" />
        <span className="min-w-0 flex-1">
          <span className="line-clamp-2 text-sm leading-snug font-bold">{summary.title}</span>
          <span className="block truncate text-xs text-ink-soft">
            {t("card.views", {
              count: Number(entry.views),
              formatted: Number(entry.views).toLocaleString(language),
            })}
            ・{formatWatchTime(entry.watchTimeSeconds)}
          </span>
        </span>
      </Link>
    </li>
  );
}

/** One video's analytics: the shared report plus how much of the audience is still watching at each moment. */
export function VideoAnalytics({
  videoId,
  durationSeconds,
  days,
}: {
  videoId: string;
  durationSeconds: number;
  days: RangeDays;
}) {
  const { t } = useTranslation("video");
  const report = useQuery({
    queryKey: videoKeys.videoAnalytics(videoId, days),
    queryFn: () => analyticsClient().getVideoAnalytics({ videoId, range: dateRange(days) }),
    placeholderData: (previous) => previous,
  });
  if (report.isPending) return <ReportSkeleton />;
  if (report.isError) return <AnalyticsUnavailable error={report.error} />;
  const data = report.data;
  // A hundred values: the share of sessions that watched each hundredth of the video.
  const retention: ChartPoint[] = data.retention.map((share, index) => {
    const at = formatClock((index / 100) * durationSeconds);
    return { value: share, label: at, longLabel: t("studio.analytics.retentionAt", { time: at }) };
  });

  return (
    <div className="flex flex-col gap-4">
      <RealtimeCard target={{ case: "videoId", value: videoId }} />
      <ReportView report={data} stale={report.isPlaceholderData}>
        <ChartCard
          title={t("studio.analytics.retention")}
          figure={retention.length > 0 ? t("studio.analytics.retentionLead") : undefined}
        >
          {retention.length === 0 ? (
            <p className="py-6 text-center text-sm font-bold text-ink-faint">
              {t("studio.analytics.noRetention")}
            </p>
          ) : (
            <TrendChart
              title={t("studio.analytics.retention")}
              points={retention}
              max={1}
              format={(share) => `${Math.round(share * 100)}%`}
              emptyText={t("studio.analytics.noRetention")}
              positionHeader={t("studio.analytics.position")}
              keyboardLabel={t("studio.analytics.keyboard")}
            />
          )}
        </ChartCard>
      </ReportView>
    </div>
  );
}

/** Views of the last 48 hours and the last hour, refreshed every minute while the page is open. */
function RealtimeCard({ target }: { target: { case: "channelId" | "videoId"; value: string } }) {
  const { t, i18n } = useTranslation("video");
  const realtime = useQuery({
    queryKey: videoKeys.realtimeAnalytics(
      target.case === "channelId" ? "channel" : "video",
      target.value,
    ),
    queryFn: () => analyticsClient().getRealtimeAnalytics({ target }),
    refetchInterval: 60_000,
  });
  if (realtime.isPending) return <Skeleton className="h-56 rounded-card" />;
  // The period report below says what is wrong; a second apology would only be noise.
  if (realtime.isError) return null;
  const data = realtime.data;
  const now = Date.now();
  const time = (at: number) =>
    new Date(at).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });
  const hours: ChartPoint[] = data.viewsPerHour.map((views, index, all) => {
    const start = now - (all.length - 1 - index) * 3_600_000;
    const ago = all.length - 1 - index;
    return {
      value: Number(views),
      label: ago === 0 ? t("studio.analytics.now") : t("studio.analytics.hoursAgo", { count: ago }),
      longLabel: new Date(start).toLocaleString(i18n.language, {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
      }),
    };
  });
  const minutes: ChartPoint[] = data.viewsPerMinute.map((views, index, all) => {
    const ago = all.length - 1 - index;
    return {
      value: Number(views),
      label:
        ago === 0 ? t("studio.analytics.now") : t("studio.analytics.minutesAgo", { count: ago }),
      longLabel: time(now - ago * 60_000),
    };
  });
  const count = (value: number) =>
    t("card.views", { count: value, formatted: value.toLocaleString(i18n.language) });

  return (
    <ChartCard
      title={t("studio.analytics.realtime")}
      figure={
        <span className="inline-flex items-center gap-1.5">
          <span className="relative flex size-2.5" aria-hidden>
            <span className="absolute inline-flex size-full animate-pulse-ring rounded-full bg-live" />
            <span className="relative inline-flex size-2.5 rounded-full bg-live" />
          </span>
          {t("studio.analytics.live")}
        </span>
      }
    >
      <div className="grid gap-6 md:grid-cols-2" data-testid="realtime-analytics">
        <div>
          <p className="text-3xl leading-tight font-black">{formatCount(data.viewsLast48Hours)}</p>
          <p className="mb-2 text-xs font-bold text-ink-soft">
            {t("studio.analytics.last48Hours")}
          </p>
          <ColumnChart
            compact
            title={t("studio.analytics.last48Hours")}
            points={hours}
            format={count}
            emptyText={t("studio.analytics.noViewsYet")}
            positionHeader={t("studio.analytics.hour")}
            keyboardLabel={t("studio.analytics.keyboard")}
          />
        </div>
        <div>
          <p className="text-3xl leading-tight font-black">
            {formatCount(data.viewsLast60Minutes)}
          </p>
          <p className="mb-2 text-xs font-bold text-ink-soft">
            {t("studio.analytics.last60Minutes")}
          </p>
          <ColumnChart
            compact
            title={t("studio.analytics.last60Minutes")}
            points={minutes}
            format={count}
            emptyText={t("studio.analytics.noViewsYet")}
            positionHeader={t("studio.analytics.minute")}
            keyboardLabel={t("studio.analytics.keyboard")}
          />
        </div>
      </div>
    </ChartCard>
  );
}
