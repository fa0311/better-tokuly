import { useQuery } from "@tanstack/react-query";
import { errorReason, type live, toMillis } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Coins } from "#/components/ui/coins";
import { EmptyState, Page } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { accountTimeZone } from "#/lib/account-time";
import { analyticsClient, liveKeys } from "../api";
import { formatCount, formatMoment } from "../parts";
import { clock } from "../player/live-player";
import { Section, Stat } from "./shared";

/** Viewers over time as an area chart. The numbers at its ends and its peak are written out next to it. */
export function ViewerChart({ samples, label }: { samples: live.ViewerSample[]; label: string }) {
  const { i18n } = useTranslation("live");
  const values = samples.map((sample) => Number(sample.viewers));
  const peak = Math.max(1, ...values);
  const width = 600;
  const height = 120;
  const step = values.length > 1 ? width / (values.length - 1) : width;
  const points = values.map(
    (value, index) =>
      `${(index * step).toFixed(1)},${(height - (value / peak) * (height - 8)).toFixed(1)}`,
  );
  const time = (sample: live.ViewerSample | undefined) =>
    sample?.at
      ? new Date(toMillis(sample.at)).toLocaleTimeString(i18n.language, {
          hour: "2-digit",
          minute: "2-digit",
          timeZone: accountTimeZone(),
        })
      : "";
  return (
    <figure data-testid="viewer-chart">
      <figcaption className="mb-1 flex justify-between text-xs font-bold text-ink-faint">
        <span>{label}</span>
        <span className="tabular-nums">max {peak.toLocaleString(i18n.language)}</span>
      </figcaption>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        className="h-28 w-full rounded-xl bg-surface-2"
        role="img"
        aria-label={label}
      >
        <polygon
          points={`0,${height} ${points.join(" ")} ${width},${height}`}
          className="fill-brand-soft"
        />
        <polyline
          points={points.join(" ")}
          fill="none"
          className="stroke-brand"
          strokeWidth="2.5"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div className="mt-1 flex justify-between text-xs font-bold text-ink-faint tabular-nums">
        <span>{time(samples[0])}</span>
        <span>{time(samples.at(-1))}</span>
      </div>
    </figure>
  );
}

/** `/studio/live/<id>/analytics`: how a stream of the caller went. */
export function StreamAnalyticsScreen({ streamId }: { streamId: string }) {
  const { t, i18n } = useTranslation("live");
  const query = useQuery({
    queryKey: liveKeys.analytics(streamId),
    queryFn: async () => {
      try {
        return (await analyticsClient().getStreamAnalytics({ streamId })).analytics ?? null;
      } catch (error) {
        const reason = errorReason(error);
        if (reason === "ANALYTICS_UNAVAILABLE" || reason === "STREAM_NOT_FOUND") return null;
        throw error;
      }
    },
  });
  const analytics = query.data;
  const report = analytics?.report;
  const count = (value: bigint | number) => formatCount(value, i18n.language);
  return (
    <Page title={t("analytics.title")} back={`/studio/live/${streamId}`}>
      {query.isPending ? (
        <PaneSpinner />
      ) : !analytics || !report ? (
        <EmptyState
          emoji="📈"
          title={t("analytics.unavailableTitle")}
          body={t("analytics.unavailableBody")}
        />
      ) : (
        <div className="flex flex-col gap-4" data-testid="stream-analytics">
          <Section
            title={report.stream?.title ?? ""}
            lead={
              report.stream?.startedAt
                ? formatMoment(toMillis(report.stream.startedAt), i18n.language)
                : undefined
            }
          >
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <Stat label={t("analytics.duration")} value={clock(Number(report.durationSeconds))} />
              <Stat label={t("analytics.peak")} value={count(report.peakViewers)} />
              <Stat label={t("analytics.average")} value={report.averageViewers.toFixed(1)} />
              <Stat label={t("analytics.unique")} value={count(analytics.uniqueViewers)} />
              <Stat
                label={t("analytics.watchTime")}
                value={t("analytics.minutes", {
                  count: Math.round(Number(analytics.watchSeconds) / 60),
                })}
              />
              <Stat
                label={t("analytics.averageWatch")}
                value={clock(analytics.averageWatchSeconds)}
              />
              <Stat label={t("analytics.chat")} value={count(report.chatMessageCount)} />
              <Stat label={t("analytics.chatters")} value={count(analytics.uniqueChatters)} />
              <Stat label={t("analytics.likes")} value={count(report.likeCount)} />
              <Stat label={t("analytics.subscribers")} value={count(report.newSubscribers)} />
              <Stat label={t("analytics.gifts")} value={count(report.giftCount)} />
              <Stat
                label={t("analytics.giftCoins")}
                value={
                  <span className="flex flex-wrap items-baseline gap-x-2">
                    <Coins amount={report.giftCoins} />
                    <span className="text-xs font-bold text-ink-faint">
                      {t("analytics.net", { coins: count(report.giftNetCoins) })}
                    </span>
                  </span>
                }
              />
            </div>
          </Section>
          {analytics.viewerSamples.length > 1 && (
            <Section title={t("analytics.viewers")}>
              <ViewerChart samples={analytics.viewerSamples} label={t("analytics.viewersChart")} />
            </Section>
          )}
          {analytics.topSupporters.length > 0 && (
            <Section title={t("supporters.title")}>
              <ol className="flex flex-col gap-2">
                {analytics.topSupporters.map((supporter, index) => (
                  <li key={supporter.user?.id ?? index} className="flex items-center gap-3">
                    <span className="w-5 text-center font-black text-ink-faint">{index + 1}</span>
                    <Avatar user={supporter.user} size="sm" />
                    <span className="min-w-0 flex-1 truncate font-bold">
                      <EmojiText text={supporter.user?.displayName ?? t("supporters.gone")} />
                    </span>
                    <Coins amount={supporter.coins} size="sm" />
                  </li>
                ))}
              </ol>
            </Section>
          )}
        </div>
      )}
    </Page>
  );
}
