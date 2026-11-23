import { useMutation, useQueryClient } from "@tanstack/react-query";
import { quest, toMillis } from "@tokuly/api";
import { ArrowRight, Check, Gift, Shuffle } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { useOpenPath } from "#/features/engagement";
import { useBalance } from "#/features/wallet";
import { useAccountNow } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { missionClient, questKeys, useLocalized, useMissions } from "./api";
import { reveal } from "./celebrate";
import { fraction } from "./model";
import { ProgressBar, RewardLine, SectionTitle, useRemainingText } from "./parts";

/** Paths the server names for a mission that this app has under another address. */
const PATH_ALIASES: Record<string, string> = { "/friends/add": "/friends" };

const DIFFICULTY_KEY = {
  [quest.MissionDifficulty.UNSPECIFIED]: undefined,
  [quest.MissionDifficulty.EASY]: "easy",
  [quest.MissionDifficulty.NORMAL]: "normal",
  [quest.MissionDifficulty.HARD]: "hard",
} as const;

const DIFFICULTY_TINT = {
  easy: "bg-mint text-[#17122b]",
  normal: "bg-sky text-[#17122b]",
  hard: "bg-pink text-white",
} as const;

/**
 * Today's and this week's missions: progress, claiming (coins, XP, sometimes
 * a badge), the bonus for finishing the day, one reroll a day, and rewards of
 * earlier days that were never collected.
 */
export function Missions() {
  const { t } = useTranslation("quest");
  const queryClient = useQueryClient();
  const balance = useBalance();
  const { data, isPending, isError, refetch } = useMissions();
  const { ms: now } = useAccountNow(30_000);
  const left = useRemainingText();
  const [rerolling, setRerolling] = useState<quest.Mission | null>(null);

  const claim = useMutation({
    mutationFn: (mission: quest.Mission) => missionClient().claimMission({ missionId: mission.id }),
    onSuccess: (response, asked) => {
      const mission = response.mission ?? asked;
      queryClient.setQueryData<quest.ListMissionsResponse | undefined>(
        questKeys.missions,
        (known) =>
          known
            ? {
                ...known,
                daily: known.daily.map((item) => (item.id === mission.id ? mission : item)),
                weekly: known.weekly?.id === mission.id ? mission : known.weekly,
                unclaimed: known.unclaimed.filter((item) => item.id !== mission.id),
                dailyBonus: response.dailyBonus ?? known.dailyBonus,
              }
            : known,
      );
      if (response.progress) queryClient.setQueryData(questKeys.progress, response.progress);
      void queryClient.invalidateQueries({ queryKey: questKeys.missions });
      // The coins arrive as a wallet push too; asking makes the balance right even without one.
      void balance.refresh();
      if (!response.granted) return;
      const shown = reveal({
        kind: "claim",
        mission,
        reward: response.reward,
        bonus: response.bonus,
      });
      if (shown === "toast") {
        const coins = (response.reward?.coins ?? 0) + (response.bonus?.coins ?? 0);
        const xp = (response.reward?.xp ?? 0) + (response.bonus?.xp ?? 0);
        toast.success(t("missions.claimed", { coins, xp }), {
          description: response.bonus ? t("celebrate.claim.bonus") : undefined,
        });
      }
    },
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "quest" }));
      void refetch();
    },
  });

  const reroll = useMutation({
    mutationFn: (mission: quest.Mission) =>
      missionClient().rerollMission({ missionId: mission.id }),
    onSuccess: (response) => {
      const mission = response.mission;
      queryClient.setQueryData<quest.ListMissionsResponse | undefined>(
        questKeys.missions,
        (known) =>
          known && mission
            ? {
                ...known,
                daily: known.daily.map((item) => (item.id === mission.id ? mission : item)),
                weekly: known.weekly?.id === mission.id ? mission : known.weekly,
                rerollsLeft: response.rerollsLeft,
              }
            : known,
      );
      setRerolling(null);
      toast.success(t("missions.rerolled"));
    },
    onError: (error) => {
      setRerolling(null);
      toast.error(errorMessage(error, { namespace: "quest" }));
      void refetch();
    },
  });

  if (isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-28" />
        <Skeleton className="h-28" />
        <Skeleton className="h-28" />
      </div>
    );
  if (isError || !data)
    return (
      <EmptyState
        emoji="🛰️"
        title={t("missions.error")}
        action={
          <Button variant="secondary" onClick={() => void refetch()}>
            {t("common:actions.retry")}
          </Button>
        }
      />
    );

  const bonus = data.dailyBonus;
  const canReroll = data.rerollsLeft > 0;
  const card = (mission: quest.Mission) => (
    <MissionCard
      key={mission.id}
      mission={mission}
      now={now}
      claiming={claim.isPending && claim.variables?.id === mission.id}
      onClaim={() => claim.mutate(mission)}
      onReroll={canReroll ? () => setRerolling(mission) : undefined}
    />
  );

  return (
    <div className="flex flex-col gap-7">
      {data.unclaimed.length > 0 && (
        <section aria-label={t("missions.unclaimed")}>
          <SectionTitle aside={t("missions.unclaimedHint")}>{t("missions.unclaimed")}</SectionTitle>
          <div className="mt-3 flex flex-col gap-3">{data.unclaimed.map(card)}</div>
        </section>
      )}

      <section aria-label={t("missions.daily")}>
        <SectionTitle
          aside={
            data.dailyResetsAt &&
            t("missions.resetsIn", { left: left(toMillis(data.dailyResetsAt), now) })
          }
        >
          {t("missions.daily")}
        </SectionTitle>
        {data.daily.length === 0 ? (
          <EmptyState emoji="🌙" title={t("missions.none")} body={t("missions.noneBody")} />
        ) : (
          <div className="mt-3 flex flex-col gap-3">{data.daily.map(card)}</div>
        )}

        {bonus && bonus.required > 0 && (
          <div
            className={cn(
              "mt-3 flex items-center gap-3 rounded-card border-2 border-dashed border-ink px-4 py-3",
              bonus.granted ? "bg-lime text-[#17122b]" : "bg-surface",
            )}
          >
            <Gift className="size-6 shrink-0" />
            <div className="min-w-0 flex-1">
              <p className="font-black">
                {bonus.granted ? t("missions.bonusGranted") : t("missions.bonus")}
              </p>
              <p className={cn("text-sm font-bold", !bonus.granted && "text-ink-soft")}>
                {t("missions.bonusProgress", { claimed: bonus.claimed, required: bonus.required })}
              </p>
            </div>
            <RewardLine reward={bonus.reward} className="shrink-0 justify-end" />
          </div>
        )}

        <p className="mt-3 flex items-center gap-1.5 text-sm font-bold text-ink-soft">
          <Shuffle className="size-4 shrink-0" />
          {canReroll ? t("missions.rerollLeft") : t("missions.rerollUsed")}
        </p>
      </section>

      {data.weekly && (
        <section aria-label={t("missions.weekly")}>
          <SectionTitle
            aside={
              data.weeklyResetsAt &&
              t("missions.resetsIn", { left: left(toMillis(data.weeklyResetsAt), now) })
            }
          >
            {t("missions.weekly")}
          </SectionTitle>
          <div className="mt-3">{card(data.weekly)}</div>
        </section>
      )}

      <ConfirmDialog
        open={rerolling !== null}
        onOpenChange={(open) => !open && setRerolling(null)}
        title={t("missions.rerollConfirm.title")}
        description={t("missions.rerollConfirm.body")}
        confirmLabel={t("missions.rerollConfirm.action")}
        loading={reroll.isPending}
        onConfirm={() => rerolling && reroll.mutate(rerolling)}
      />
    </div>
  );
}

function MissionCard({
  mission,
  now,
  claiming,
  onClaim,
  onReroll,
}: {
  mission: quest.Mission;
  now: number;
  claiming: boolean;
  onClaim(): void;
  /** Unset when the day's reroll is used. */
  onReroll?: () => void;
}) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const openPath = useOpenPath();
  const left = useRemainingText();
  const title = text(mission.title);
  const difficulty = DIFFICULTY_KEY[mission.difficulty];
  const active = mission.state === quest.MissionState.ACTIVE;
  const completed = mission.state === quest.MissionState.COMPLETED;
  const claimed = mission.state === quest.MissionState.CLAIMED;
  const expired = mission.state === quest.MissionState.EXPIRED;
  // A mission of an earlier period keeps its place in the list until it is claimed; it cannot be swapped.
  const current = mission.endsAt ? toMillis(mission.endsAt) > now : false;
  const progress = t("missions.progress", {
    progress: Number(mission.progress).toLocaleString(),
    target: Number(mission.target).toLocaleString(),
  });

  return (
    <article
      aria-label={title}
      data-state={claimed ? "claimed" : completed ? "completed" : expired ? "expired" : "active"}
      className={cn(
        "sticker rounded-card p-4",
        completed ? "bg-brand-soft" : "bg-surface",
        (claimed || expired) && "opacity-70",
      )}
    >
      <div className="flex items-start gap-3">
        <span
          className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-surface-2 text-2xl"
          aria-hidden
        >
          {mission.emoji || "🎯"}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <h3 className="min-w-0 font-black leading-snug">{title}</h3>
            {difficulty && (
              <span
                className={cn(
                  "rounded-full px-2 py-0.5 text-[11px] font-black",
                  DIFFICULTY_TINT[difficulty],
                )}
              >
                {t(`missions.difficulty.${difficulty}`)}
              </span>
            )}
          </div>
          <p className="mt-0.5 text-sm text-ink-soft">{text(mission.description)}</p>
        </div>
        {active && current && onReroll && (
          <button
            type="button"
            onClick={onReroll}
            aria-label={t("missions.reroll", { title })}
            title={t("missions.rerollShort")}
            className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
          >
            <Shuffle className="size-[18px]" />
          </button>
        )}
      </div>

      <div className="mt-3 flex items-center gap-3">
        <ProgressBar
          className="flex-1"
          value={claimed || completed ? 1 : fraction(mission.progress, mission.target)}
          label={t("missions.progressLabel", { title })}
          valueText={progress}
          tone={claimed || completed ? "lime" : "brand"}
        />
        <span className="shrink-0 text-sm font-black tabular-nums">{progress}</span>
      </div>

      <div className="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <RewardLine reward={mission.reward} />
        {completed ? (
          <Button
            variant="lime"
            size="sm"
            loading={claiming}
            onClick={onClaim}
            icon={<Gift className="size-4" />}
          >
            {t("missions.claim")}
          </Button>
        ) : claimed ? (
          <span className="inline-flex items-center gap-1 text-sm font-black text-success">
            <Check className="size-4" />
            {t("missions.claimedState")}
          </span>
        ) : expired ? (
          <span className="text-sm font-bold text-ink-faint">{t("missions.expired")}</span>
        ) : mission.path ? (
          <button
            type="button"
            onClick={() => openPath(PATH_ALIASES[mission.path] ?? mission.path)}
            className="inline-flex h-9 items-center gap-1 rounded-full px-3 text-sm font-black text-brand hover:bg-surface-2"
          >
            {t("missions.go")}
            <ArrowRight className="size-4" />
          </button>
        ) : (
          mission.endsAt && (
            <span className="text-sm font-bold text-ink-faint">
              {t("missions.endsIn", { left: left(toMillis(mission.endsAt), now) })}
            </span>
          )
        )}
      </div>
    </article>
  );
}
