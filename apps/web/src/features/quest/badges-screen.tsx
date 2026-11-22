import { Link } from "@tanstack/react-router";
import type { quest } from "@tokuly/api";
import { Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useBadgeCatalog, useLocalized, useUserBadges } from "./api";
import { badgeLines, fraction, isHiddenSecret, nextInLine } from "./model";
import { BadgeMedal, ProgressBar, QuestNav, RarityChip, TitlePill, useBadgeName } from "./parts";

/**
 * `/quest/badges`: the whole collection by family. Tiers of one achievement
 * sit side by side; what is not earned yet is greyed with how far it is, and
 * secret badges stay question marks until they are earned.
 */
export function BadgesScreen() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const { data, isPending, isError, refetch } = useBadgeCatalog();
  const mine = useUserBadges("");

  return (
    <Page title={t("badges.title")} wide back="/quest">
      <QuestNav className="lg:max-w-xl" />
      {isPending ? (
        <div className="grid gap-4 lg:grid-cols-2">
          <Skeleton className="h-56" />
          <Skeleton className="h-56" />
        </div>
      ) : isError || !data ? (
        <EmptyState
          emoji="🛰️"
          title={t("badges.error")}
          action={
            <Button variant="secondary" onClick={() => void refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : (
        <>
          <section
            aria-label={t("badges.showcase")}
            className="sticker flex flex-wrap items-center gap-x-5 gap-y-3 rounded-card bg-surface p-4"
          >
            <div className="min-w-0 flex-1">
              <p className="text-2xl font-black tabular-nums" data-testid="quest-badge-count">
                {t("badges.count", { owned: data.ownedCount, total: data.totalCount })}
              </p>
              <div className="mt-1.5 flex min-h-10 flex-wrap items-center gap-2">
                {mine.data?.showcase.length ? (
                  mine.data.showcase.map((badge) => (
                    <BadgeMedal key={badge.key} badge={badge} owned size="sm" />
                  ))
                ) : (
                  <p className="text-sm font-bold text-ink-soft">{t("badges.showcaseEmpty")}</p>
                )}
                {mine.data?.title && <TitlePill title={text(mine.data.title.text)} />}
              </div>
            </div>
            <Link
              to="/quest/showcase"
              className="pressable sticker-sm inline-flex h-10 shrink-0 items-center gap-1.5 rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
            >
              <Sparkles className="size-4" />
              {t("badges.editShowcase")}
            </Link>
          </section>

          <div className="mt-6 grid gap-5 lg:grid-cols-2 lg:items-start">
            {data.families.map((family) => (
              <Family key={family.key} family={family} />
            ))}
          </div>
        </>
      )}
    </Page>
  );
}

function Family({ family }: { family: quest.BadgeFamily }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const name = text(family.name);
  return (
    <section aria-label={name} className="sticker rounded-card bg-surface p-4">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-black">
          <span aria-hidden>{family.emoji}</span> {name}
        </h2>
        <span className="text-sm font-black text-ink-soft tabular-nums">
          {t("badges.familyCount", { owned: family.ownedCount, total: family.badges.length })}
        </span>
      </div>
      <ul className="mt-3 flex flex-col gap-1">
        {badgeLines(family.badges).map((line) => (
          <Line key={line[0]?.key} line={line} />
        ))}
      </ul>
    </section>
  );
}

/** One achievement: its tiers as medals, its name, and the way to the next tier. */
function Line({ line }: { line: quest.Badge[] }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const next = nextInLine(line);
  if (!next) return null;
  const secret = isHiddenSecret(next);
  const ownedAll = line.every((badge) => badge.owned);
  const counted = next.target > 0n && !next.owned;
  const name = text(next.name) || t("badges.secretName");
  return (
    <li className="flex items-center gap-3 rounded-2xl px-1 py-2">
      <div className="flex shrink-0 items-center gap-1.5">
        {line.map((badge) => (
          <Link
            key={badge.key}
            to="/quest/badges/$key"
            params={{ key: badge.key }}
            aria-label={t(badge.owned ? "badges.openOwned" : "badges.openLocked", {
              name: badgeName(badge),
            })}
            className="rounded-full pb-1.5 transition-transform hover:scale-105"
          >
            <BadgeMedal
              badge={badge}
              size={line.length > 1 ? "sm" : "md"}
              className={cn(line.length > 1 && "mb-0.5")}
            />
          </Link>
        ))}
      </div>
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-x-2 gap-y-0.5 font-black leading-snug">
          <span className="min-w-0">{name}</span>
          {!secret && <RarityChip rarity={next.rarity} className="h-5 px-2 text-[10px]" />}
        </p>
        <p className="text-sm leading-snug text-ink-soft">
          {secret ? t("badges.secretHint") : text(next.description)}
        </p>
        {counted && (
          <div className="mt-1.5 flex items-center gap-2">
            <ProgressBar
              className="h-2 flex-1 border-0 bg-surface-3"
              value={fraction(next.progress, next.target)}
              label={t("badges.progressLabel", { name })}
              valueText={`${next.progress} / ${next.target}`}
            />
            <span className="shrink-0 text-xs font-black text-ink-soft tabular-nums">
              {Number(next.progress).toLocaleString()} / {Number(next.target).toLocaleString()}
            </span>
          </div>
        )}
        {ownedAll && !secret && (
          <p className="mt-0.5 text-xs font-black text-success">{t("badges.complete")}</p>
        )}
      </div>
    </li>
  );
}
