import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { shorts } from "@tokuly/api";
import { imageUrl } from "@tokuly/core/shorts";
import type { TFunction } from "i18next";
import { ChevronRight, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { EmojiText } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { useMyChannels } from "#/features/video";
import { shiftDate, useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { analyticsClient, shortKeys, useShort } from "../api";
import { ChoiceChips } from "../sort-chips";
import { type AnalyticsSearch, RANGES } from "./analytics-search";

const { PlaybackSource } = shorts;

const SOURCE_KEYS: Record<number, string> = {
  [PlaybackSource.FOR_YOU]: "forYou",
  [PlaybackSource.FOLLOWING]: "following",
  [PlaybackSource.FRIENDS]: "friends",
  [PlaybackSource.PROFILE]: "profile",
  [PlaybackSource.HASHTAG]: "hashtag",
  [PlaybackSource.SHARE_LINK]: "shareLink",
  [PlaybackSource.SOUND]: "sound",
  [PlaybackSource.SEARCH]: "search",
  [PlaybackSource.NOTIFICATION]: "notification",
};

/** The counts a creator can chart by day. */
const DAILY = ["views", "likes", "comments", "shares", "saves", "followersGained"] as const;
type DailyMetric = (typeof DAILY)[number];

interface Report {
  totals?: shorts.ShortAnalyticsTotals;
  daily: shorts.DailyShortAnalytics[];
  trafficSources: shorts.PlaybackSourceStats[];
  topShorts?: shorts.ShortStats[];
  shortCount?: bigint;
}

/**
 * `/shorts/studio/analytics`: how the shorts of a channel — or one short —
 * are doing over a range of days: counts as they are now, how people
 * watched (once enough of them did), and where they came from.
 */
export function AnalyticsScreen({
  search,
  onSearchChange,
}: {
  search: AnalyticsSearch;
  onSearchChange(next: AnalyticsSearch): void;
}) {
  const { t } = useTranslation("shorts");
  const channelsQuery = useMyChannels();
  const channels = channelsQuery.data?.channels ?? [];
  const [channelId, setChannelId] = useState("");
  const channel = channels.find((entry) => entry.summary?.id === channelId) ?? channels[0];
  const days = search.days ?? 28;
  const short = useShort(search.short);
  // Days are the channel owner's days: the account's calendar, not the device's.
  const today = useAccountToday();
  const start = shiftDate(today, -(days - 1));
  const range = { startDate: start, endDate: today };
  const subjectId = search.short ? (short.data?.id ?? "") : (channel?.summary?.id ?? "");

  const report = useQuery({
    queryKey: shortKeys.analytics(search.short ? "short" : "channel", subjectId, start, today),
    queryFn: (): Promise<Report> =>
      search.short
        ? analyticsClient().getShortAnalytics({ shortId: subjectId, range })
        : analyticsClient().getChannelShortsAnalytics({ channelId: subjectId, range }),
    enabled: Boolean(subjectId),
  });

  return (
    <Page title={t("analytics.title")} back="/shorts/studio" wide>
      {channelsQuery.isPending ? (
        <Skeleton className="h-96 rounded-card" />
      ) : channels.length === 0 ? (
        <EmptyState
          emoji="📊"
          title={t("create.needChannel")}
          action={
            <Link
              to="/studio/channels/new"
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              <Plus className="size-4" />
              {t("create.makeChannel")}
            </Link>
          }
        />
      ) : (
        <div className="flex flex-col gap-4">
          {search.short ? (
            <section className="sticker flex items-center gap-3 rounded-card bg-surface p-3">
              <span className="relative block aspect-[9/16] w-12 shrink-0 overflow-hidden rounded-xl border-2 border-ink bg-surface-3">
                {short.data?.cover && (
                  <img
                    src={imageUrl(short.data.cover, 48)}
                    alt=""
                    className="size-full object-cover"
                  />
                )}
              </span>
              <p className="line-clamp-2 min-w-0 flex-1 font-bold">
                <EmojiText text={short.data?.caption || t("studio.noCaption")} />
              </p>
              <button
                type="button"
                className="pressable sticker-sm h-9 shrink-0 rounded-full bg-surface px-4 text-sm font-bold"
                onClick={() => onSearchChange({ days: search.days })}
              >
                {t("analytics.wholeChannel")}
              </button>
            </section>
          ) : (
            channels.length > 1 && (
              <SelectField
                label={t("form.channel")}
                value={channel?.summary?.id ?? ""}
                onChange={setChannelId}
                options={channels.map((entry) => ({
                  value: entry.summary?.id ?? "",
                  label: entry.summary?.name ?? "",
                }))}
              />
            )
          )}
          <ChoiceChips
            label={t("analytics.rangeLabel")}
            value={days}
            onChange={(next) => onSearchChange({ ...search, days: next === 28 ? undefined : next })}
            options={RANGES.map((value) => ({
              value,
              label: t("analytics.range", { count: value }),
            }))}
          />
          {report.isError || (search.short && short.isError) ? (
            <EmptyState
              emoji="😵‍💫"
              title={errorMessage(report.error ?? short.error, { namespace: "shorts" })}
            />
          ) : !report.data ? (
            <Skeleton className="h-96 rounded-card" />
          ) : (
            <ReportView
              report={report.data}
              onOpenShort={(shortId) => onSearchChange({ ...search, short: shortId })}
            />
          )}
        </div>
      )}
    </Page>
  );
}

function Tile({ label, value, testId }: { label: string; value: string; testId?: string }) {
  return (
    <div className="rounded-2xl border-2 border-line bg-surface px-3 py-2.5">
      <div className="text-xs font-bold text-ink-soft">{label}</div>
      <div className="mt-0.5 text-2xl font-black tabular-nums" data-testid={testId}>
        {value}
      </div>
    </div>
  );
}

function watchTime(seconds: number, t: TFunction<"shorts">): string {
  const minutes = Math.round(seconds / 60);
  return minutes >= 60
    ? t("analytics.hours", { hours: (minutes / 60).toFixed(1) })
    : minutes >= 1
      ? t("analytics.minutes", { minutes })
      : t("analytics.seconds", { seconds: Math.round(seconds) });
}

function ReportView({
  report,
  onOpenShort,
}: {
  report: Report;
  onOpenShort(shortId: string): void;
}) {
  const { t, i18n } = useTranslation("shorts");
  const [metric, setMetric] = useState<DailyMetric>("views");
  const totals = report.totals;
  const count = (value: bigint | undefined) => compactCount(Number(value ?? 0n), i18n.language);
  // How people watched is only reported for finished days with enough viewers: until then there is nothing to show, not a zero.
  const watched = Boolean(totals && totals.plays > 0n);
  const values = report.daily.map((day) => Number(day[metric]));
  const peak = Math.max(1, ...values);
  const sourceTotal = report.trafficSources.reduce((sum, entry) => sum + Number(entry.views), 0);

  return (
    <>
      <section className="sticker rounded-card bg-surface p-4" aria-labelledby="analytics-counts">
        <h2 id="analytics-counts" className="text-lg font-black">
          {t("analytics.counts")}
        </h2>
        <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
          <Tile
            label={t("analytics.metric.views")}
            value={count(totals?.views)}
            testId="analytics-views"
          />
          <Tile label={t("analytics.metric.uniqueViewers")} value={count(totals?.uniqueViewers)} />
          <Tile label={t("analytics.metric.likes")} value={count(totals?.likes)} />
          <Tile label={t("analytics.metric.comments")} value={count(totals?.comments)} />
          <Tile label={t("analytics.metric.shares")} value={count(totals?.shares)} />
          <Tile label={t("analytics.metric.saves")} value={count(totals?.saves)} />
          <Tile
            label={t("analytics.metric.followersGained")}
            value={count(totals?.followersGained)}
          />
          {report.shortCount !== undefined && (
            <Tile label={t("analytics.metric.shortCount")} value={count(report.shortCount)} />
          )}
        </div>
      </section>

      <section className="sticker rounded-card bg-surface p-4" aria-labelledby="analytics-watch">
        <h2 id="analytics-watch" className="text-lg font-black">
          {t("analytics.watch")}
        </h2>
        {watched && totals ? (
          <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
            <Tile
              label={t("analytics.metric.watchRatio")}
              value={`${Math.round(totals.averageWatchRatio * 100)}%`}
            />
            <Tile label={t("analytics.metric.loops")} value={count(totals.loops)} />
            <Tile
              label={t("analytics.metric.watchTime")}
              value={watchTime(totals.watchTimeSeconds, t)}
            />
            <Tile label={t("analytics.metric.plays")} value={count(totals.plays)} />
          </div>
        ) : (
          <p
            className="mt-3 rounded-2xl border-2 border-dashed border-line px-4 py-5 text-center font-bold text-ink-soft"
            data-testid="analytics-not-enough"
          >
            {t("analytics.notEnough")}
          </p>
        )}
        <p className="mt-3 text-sm text-ink-soft">{t("analytics.watchNote")}</p>
      </section>

      <section className="sticker rounded-card bg-surface p-4" aria-labelledby="analytics-daily">
        <h2 id="analytics-daily" className="text-lg font-black">
          {t("analytics.daily")}
        </h2>
        <ChoiceChips
          className="mt-3"
          label={t("analytics.metricLabel")}
          value={metric}
          onChange={(next) => setMetric(next)}
          options={DAILY.map((value) => ({ value, label: t(`analytics.metric.${value}`) }))}
        />
        <ol
          className="mt-4 flex h-40 items-end gap-px"
          aria-label={t("analytics.dailyLabel", { metric: t(`analytics.metric.${metric}`) })}
        >
          {report.daily.map((day, index) => {
            const value = values[index] ?? 0;
            return (
              <li
                key={day.date}
                className="group relative flex h-full min-w-0 flex-1 items-end"
                title={`${day.date}: ${value.toLocaleString(i18n.language)}`}
              >
                <span className="sr-only">
                  {day.date}: {value}
                </span>
                <span
                  aria-hidden
                  className={cn(
                    "w-full rounded-t-sm bg-brand group-hover:bg-brand-strong",
                    value === 0 && "bg-line",
                  )}
                  style={{ height: `${Math.max(2, (value / peak) * 100)}%` }}
                />
              </li>
            );
          })}
        </ol>
        <div className="mt-1.5 flex justify-between text-xs font-bold text-ink-faint">
          <span>{report.daily[0]?.date.slice(5).replace("-", "/")}</span>
          <span>{report.daily[report.daily.length - 1]?.date.slice(5).replace("-", "/")}</span>
        </div>
      </section>

      <section className="sticker rounded-card bg-surface p-4" aria-labelledby="analytics-sources">
        <h2 id="analytics-sources" className="text-lg font-black">
          {t("analytics.sources")}
        </h2>
        {report.trafficSources.length === 0 ? (
          <p className="mt-3 text-sm text-ink-soft">{t("analytics.noSources")}</p>
        ) : (
          <ul className="mt-3 flex flex-col gap-2.5" data-testid="analytics-sources">
            {report.trafficSources.map((entry) => {
              const views = Number(entry.views);
              const share = sourceTotal > 0 ? views / sourceTotal : 0;
              return (
                <li key={entry.source}>
                  <div className="flex justify-between gap-3 text-sm font-bold">
                    <span>{t(`analytics.source.${SOURCE_KEYS[entry.source] ?? "other"}`)}</span>
                    <span className="tabular-nums text-ink-soft">
                      {views.toLocaleString(i18n.language)} · {Math.round(share * 100)}%
                    </span>
                  </div>
                  <div className="mt-1 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-3">
                    <div className="h-full bg-lime" style={{ width: `${share * 100}%` }} />
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {report.topShorts && report.topShorts.length > 0 && (
        <section className="sticker rounded-card bg-surface p-4" aria-labelledby="analytics-top">
          <h2 id="analytics-top" className="text-lg font-black">
            {t("analytics.top")}
          </h2>
          <ol className="mt-2 flex flex-col">
            {report.topShorts.map((entry, index) =>
              entry.short ? (
                <li key={entry.short.id}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left hover:bg-surface-2"
                    onClick={() => onOpenShort(entry.short?.shortId ?? "")}
                  >
                    <span className="w-5 shrink-0 text-center font-black text-ink-faint">
                      {index + 1}
                    </span>
                    <span className="relative block aspect-[9/16] w-9 shrink-0 overflow-hidden rounded-lg border-2 border-ink bg-surface-3">
                      {entry.short.cover && (
                        <img
                          src={imageUrl(entry.short.cover, 36)}
                          alt=""
                          className="size-full object-cover"
                        />
                      )}
                    </span>
                    <span className="min-w-0 flex-1 truncate font-bold">
                      <EmojiText text={entry.short.caption || t("studio.noCaption")} />
                    </span>
                    <span className="shrink-0 text-sm font-black tabular-nums text-ink-soft">
                      {t("analytics.viewsCount", { count: count(entry.views) })}
                    </span>
                    <ChevronRight className="size-4 shrink-0 text-ink-faint" aria-hidden />
                  </button>
                </li>
              ) : null,
            )}
          </ol>
        </section>
      )}
    </>
  );
}
