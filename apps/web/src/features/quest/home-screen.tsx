import { Link } from "@tanstack/react-router";
import type { quest } from "@tokuly/api";
import { fraction, levelFraction, xpToNextLevel } from "@tokuly/core/quest";
import { ChevronRight, Settings2 } from "lucide-react";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { BalancePill } from "#/features/wallet";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useLocalized, useProgress } from "./api";
import { Missions } from "./missions";
import { ProgressBar, QuestNav, SectionTitle, TitlePill } from "./parts";

/**
 * `/quest`: where a player stands (level, XP, title), what gives XP today
 * and how much room each source still has, and the missions.
 */
export function QuestHomeScreen() {
  const { t } = useTranslation("quest");
  return (
    <Page
      title={t("title")}
      wide
      actions={
        <>
          <BalancePill />
          <Link
            to="/quest/settings"
            aria-label={t("settings.title")}
            className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
          >
            <Settings2 className="size-5" />
          </Link>
        </>
      }
    >
      <QuestNav className="lg:max-w-xl" />
      <div className="grid gap-7 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)] lg:items-start">
        <div className="flex flex-col gap-7">
          <LevelCard />
          <div className="hidden lg:block">
            <TodaySources />
          </div>
        </div>
        <Missions />
        <div className="lg:hidden">
          <TodaySources />
        </div>
      </div>
    </Page>
  );
}

/** The player's level with the bar to the next one. */
function LevelCard() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const { data: progress, isPending, isError, refetch } = useProgress();

  if (isPending) return <Skeleton className="h-56" />;
  if (isError || !progress?.level)
    return (
      <EmptyState
        emoji="🛰️"
        title={t("home.error")}
        action={
          <Button variant="secondary" onClick={() => void refetch()}>
            {t("common:actions.retry")}
          </Button>
        }
      />
    );

  const level = progress.level;
  const title = text(progress.title?.text);
  const into = Number(level.xpIntoLevel).toLocaleString();
  const needed = Number(level.xpForNextLevel).toLocaleString();
  return (
    <section
      aria-label={t("home.level")}
      className="sticker overflow-hidden rounded-card bg-surface"
    >
      <div className="bg-sun p-5 text-[#17122b]">
        <div className="flex items-center gap-4">
          <span
            className="grid size-20 shrink-0 place-items-center rounded-full border-2 border-ink bg-surface text-ink shadow-[0_3px_0_0_var(--ink)]"
            role="img"
            aria-label={t("level.label", { level: level.level })}
          >
            <span className="text-center leading-none">
              <span className="block text-[11px] font-black">Lv.</span>
              <span className="block text-4xl font-black tabular-nums" data-testid="quest-level">
                {level.level}
              </span>
            </span>
          </span>
          <div className="min-w-0 flex-1">
            {title ? (
              <TitlePill title={title} className="h-6 bg-surface px-2 text-xs" />
            ) : (
              <Link to="/quest/showcase" className="text-sm font-black underline">
                {t("home.pickTitle")}
              </Link>
            )}
            <p className="mt-1.5 text-2xl leading-tight font-black tabular-nums">
              {t("home.totalXp", { xp: Number(level.totalXp).toLocaleString() })}
            </p>
            <p className="text-sm font-bold">
              {t("home.toNext", {
                xp: xpToNextLevel(level).toLocaleString(),
                level: level.level + 1,
              })}
            </p>
          </div>
        </div>
        <ProgressBar
          className="mt-4 h-4 bg-surface"
          value={levelFraction(level)}
          label={t("home.levelBar")}
          valueText={t("home.levelBarValue", { into, needed })}
        />
        <p
          className="mt-1.5 text-right text-xs font-black tabular-nums"
          data-testid="quest-xp-into"
        >
          {t("home.levelBarValue", { into, needed })}
        </p>
      </div>
      <dl className="grid grid-cols-3 divide-x-2 divide-line border-t-2 border-ink text-center">
        <Stat label={t("home.today")} value={t("xp.amount", { xp: progress.xpToday })} />
        <Stat
          label={t("home.badges")}
          value={progress.badgeCount.toLocaleString()}
          to="/quest/badges"
        />
        <Stat label={t("home.activeDays")} value={t("home.days", { count: progress.activeDays })} />
      </dl>
    </section>
  );
}

function Stat({ label, value, to }: { label: string; value: string; to?: "/quest/badges" }) {
  const body = (
    <>
      <dd className="text-lg font-black tabular-nums">{value}</dd>
      <dt className="text-xs font-bold text-ink-faint">{label}</dt>
    </>
  );
  return to ? (
    <Link to={to} className="flex flex-col-reverse px-2 py-3 transition-colors hover:bg-surface-2">
      {body}
    </Link>
  ) : (
    <div className="flex flex-col-reverse px-2 py-3">{body}</div>
  );
}

/** What gave XP today, source by source, with how much each may still give. */
function TodaySources() {
  const { t } = useTranslation("quest");
  const { data: progress } = useProgress();
  if (!progress) return null;
  // What earned something first; sources that are full go last among those.
  const sources = [...progress.sources].sort(
    (a, b) => Number(b.xpToday > 0) - Number(a.xpToday > 0),
  );
  return (
    <section aria-label={t("sources.title")}>
      <SectionTitle
        aside={
          <Link to="/quest/xp" className="inline-flex items-center text-brand">
            {t("sources.howTo")}
            <ChevronRight className="size-4" />
          </Link>
        }
      >
        {t("sources.title")}
      </SectionTitle>
      <ul className="sticker mt-3 grid gap-x-5 gap-y-3 rounded-card bg-surface p-4 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
        {sources.map((item) => (
          <SourceRow key={item.source?.key} item={item} />
        ))}
      </ul>
    </section>
  );
}

function SourceRow({ item }: { item: quest.SourceProgress }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const source = item.source;
  if (!source) return null;
  const name = text(source.name);
  const cap = source.dailyCap;
  const value =
    cap > 0 ? t("sources.ofCap", { xp: item.xpToday, cap }) : t("xp.amount", { xp: item.xpToday });
  return (
    <li className="min-w-0">
      <div className="flex items-center gap-2 text-sm font-bold">
        <span aria-hidden>{source.emoji}</span>
        <span className="min-w-0 flex-1 truncate">{name}</span>
        {item.capped && (
          <span className="rounded-full bg-lime px-1.5 py-0.5 text-[10px] font-black text-[#17122b]">
            {t("sources.capped")}
          </span>
        )}
        <span
          className={cn("shrink-0 tabular-nums", item.xpToday > 0 ? "text-ink" : "text-ink-faint")}
        >
          {value}
        </span>
      </div>
      {cap > 0 && (
        <ProgressBar
          className="mt-1 h-2 border-0 bg-surface-3"
          value={fraction(item.xpToday, cap)}
          label={t("sources.bar", { name })}
          valueText={value}
          tone={item.capped ? "lime" : "brand"}
        />
      )}
    </li>
  );
}
