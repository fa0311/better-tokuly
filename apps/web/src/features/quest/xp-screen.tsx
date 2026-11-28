import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { quest } from "@tokuly/api";
import { formatDay, questKeys } from "@tokuly/core/quest";
import { Coins } from "#/components/ui/coins";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { shiftDate, useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { questClient, useLocalized, useProgress } from "./api";
import { SectionTitle } from "./parts";

export type XpTab = "rules" | "history";

/**
 * `/quest/xp`: how XP is earned (every rule under the daily cap it counts
 * against, the level curve and what a level-up pays) and what was earned
 * day by day.
 */
export function XpScreen({ tab, onTab }: { tab: XpTab; onTab(tab: XpTab): void }) {
  const { t } = useTranslation("quest");
  return (
    <Page title={t("xpGuide.title")} back="/quest">
      <Tabs
        className="mb-5"
        items={[
          { value: "rules", label: t("xpGuide.tabs.rules") },
          { value: "history", label: t("xpGuide.tabs.history") },
        ]}
        value={tab}
        onChange={onTab}
      />
      {tab === "rules" ? <Rules /> : <History />}
    </Page>
  );
}

function Rules() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const { data: progress } = useProgress();
  const { data, isPending, isError } = useQuery({
    queryKey: questKeys.rules,
    queryFn: () => questClient().listEarningRules({}),
    staleTime: 10 * 60_000,
  });

  if (isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-32" />
        <Skeleton className="h-32" />
      </div>
    );
  if (isError || !data) return <EmptyState emoji="🛰️" title={t("xpGuide.error")} />;

  const today = new Map(progress?.sources.map((item) => [item.source?.key ?? "", item]));
  const curve = data.curve;
  const level = progress?.level?.level ?? 1;
  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm leading-relaxed text-ink-soft">{t("xpGuide.intro")}</p>

      {data.sources.map((source) => {
        const rules = data.rules.filter((rule) => rule.source === source.key);
        if (rules.length === 0) return null;
        const earned = today.get(source.key);
        return (
          <section key={source.key} aria-label={text(source.name)}>
            <SectionTitle
              aside={
                source.dailyCap > 0
                  ? t("xpGuide.cap", { xp: earned?.xpToday ?? 0, cap: source.dailyCap })
                  : t("xpGuide.noCap")
              }
            >
              <span aria-hidden>{source.emoji}</span> {text(source.name)}
            </SectionTitle>
            <ul className="sticker mt-2 divide-y-2 divide-line rounded-card bg-surface px-4">
              {rules.map((rule) => (
                <RuleRow key={rule.key} rule={rule} />
              ))}
            </ul>
          </section>
        );
      })}

      {curve && (
        <section aria-label={t("xpGuide.levels")}>
          <SectionTitle>{t("xpGuide.levels")}</SectionTitle>
          <p className="mt-1 text-sm leading-relaxed text-ink-soft">
            {t("xpGuide.curve", { first: curve.firstLevelXp, step: curve.stepXp })}
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-sm font-bold">
            <span className="inline-flex items-center gap-1.5">
              {t("xpGuide.levelUpCoins")} <Coins amount={curve.levelUpCoins} size="sm" signed />
            </span>
            <span className="inline-flex items-center gap-1.5">
              {t("xpGuide.milestoneCoins")}{" "}
              <Coins amount={curve.milestoneLevelCoins} size="sm" signed />
            </span>
          </div>
          <ol className="sticker mt-3 grid grid-cols-2 gap-x-4 rounded-card bg-surface p-4 text-sm sm:grid-cols-3">
            {curve.thresholds.slice(0, 30).map((xp, index) => {
              const reached = index + 2;
              return (
                <li
                  key={reached}
                  className={cn(
                    "flex justify-between gap-2 py-1 font-bold tabular-nums",
                    reached <= level ? "text-ink-faint" : reached === level + 1 && "text-brand",
                  )}
                >
                  <span>{t("level.short", { level: reached })}</span>
                  <span>{t("xp.amount", { xp: Number(xp).toLocaleString() })}</span>
                </li>
              );
            })}
          </ol>
        </section>
      )}
    </div>
  );
}

function RuleRow({ rule }: { rule: quest.EarningRule }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const unit = text(rule.perUnit);
  return (
    <li className={cn("flex items-center gap-3 py-2.5", !rule.live && "text-ink-faint")}>
      <span className="min-w-0 flex-1">
        <span className="block font-bold">{text(rule.text)}</span>
        {(rule.countsForStreak || !rule.live) && (
          <span className="flex flex-wrap gap-x-2 text-xs font-bold text-ink-faint">
            {rule.countsForStreak && <span>🔥 {t("xpGuide.countsForStreak")}</span>}
            {!rule.live && <span>{t("xpGuide.notLive")}</span>}
          </span>
        )}
      </span>
      <span className="shrink-0 font-black tabular-nums">
        {unit
          ? t("xpGuide.perUnit", { xp: rule.xp, unit })
          : rule.xp > 0
            ? t("xp.plus", { xp: rule.xp })
            : "—"}
      </span>
    </li>
  );
}

function History() {
  const { t, i18n } = useTranslation("quest");
  const text = useLocalized();
  const today = useAccountToday();
  const { data: progress } = useProgress();
  const query = useInfiniteQuery({
    queryKey: questKeys.history,
    queryFn: ({ pageParam }) =>
      questClient().listXpHistory({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => (last.page?.hasMore ? last.page.nextCursor : undefined),
  });

  if (query.isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-16" />
        <Skeleton className="h-16" />
      </div>
    );
  const days = query.data?.pages.flatMap((page) => page.days) ?? [];
  if (query.isError || days.length === 0)
    return <EmptyState emoji="🌱" title={t("history.empty")} body={t("history.emptyBody")} />;

  const sources = new Map(progress?.sources.map((item) => [item.source?.key ?? "", item.source]));
  const most = Math.max(1, ...days.map((day) => day.xp));
  const label = (date: string) =>
    date === today
      ? t("history.today")
      : date === shiftDate(today, -1)
        ? t("history.yesterday")
        : formatDay(date, i18n.language, date.slice(0, 4) !== today.slice(0, 4));
  return (
    <>
      <ol className="sticker divide-y-2 divide-line rounded-card bg-surface px-4">
        {days.map((day) => (
          <li key={day.date} className="py-3">
            <div className="flex items-baseline justify-between gap-3">
              <span className="font-black">{label(day.date)}</span>
              <span className="font-black text-brand tabular-nums">
                {t("xp.plus", { xp: day.xp })}
              </span>
            </div>
            <div className="mt-1.5 h-2 overflow-hidden rounded-full bg-surface-3" aria-hidden>
              <div
                className="h-full rounded-full bg-brand"
                style={{ width: `${(day.xp / most) * 100}%` }}
              />
            </div>
            <p className="mt-1.5 flex flex-wrap gap-x-3 gap-y-0.5 text-xs font-bold text-ink-soft">
              {day.sources.map((item) => {
                const source = sources.get(item.source);
                return (
                  <span key={item.source}>
                    {source?.emoji} {text(source?.name) || item.source} {item.xp}
                  </span>
                );
              })}
            </p>
          </li>
        ))}
      </ol>
      <LoadMore query={query} />
    </>
  );
}
