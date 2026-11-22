import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { errorReason, type quest, toMillis } from "@tokuly/api";
import { MessageCircle } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { openDirectChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { BalancePill, useBalance } from "#/features/wallet";
import { useAccountNow, useAccountTimeZone } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { questKeys, STREAK_PAGE_SIZE, streakClient } from "./api";
import { formatDay, fraction, type StreakStatus, streakStatus } from "./model";
import { ProgressBar, QuestNav, SectionTitle, useRemainingText } from "./parts";

const MILESTONES = [3, 7, 30, 100, 365, 500, 1000];

/**
 * `/quest/streaks`: every friend streak 🔥 — how many days, whose turn it is
 * today, the hourglass when one is about to end, the way to the next
 * milestone — and bringing back one that ended within the last day, for
 * coins.
 */
export function StreaksScreen() {
  const { t, i18n } = useTranslation("quest");
  const queryClient = useQueryClient();
  const balance = useBalance();
  const navigate = useNavigate();
  const zone = useAccountTimeZone();
  const { ms: now } = useAccountNow(30_000);
  const [restoring, setRestoring] = useState<quest.Streak | null>(null);

  const query = useInfiniteQuery({
    queryKey: [...questKeys.streaks, "pages"],
    queryFn: ({ pageParam }) =>
      streakClient().listStreaks({ page: { cursor: pageParam, limit: STREAK_PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => (last.page?.hasMore ? last.page.nextCursor : undefined),
  });

  const restore = useMutation({
    mutationFn: (streak: quest.Streak) =>
      streakClient().restoreStreak({ userId: streak.friend?.id ?? "" }),
    onSuccess: (response, streak) => {
      setRestoring(null);
      void queryClient.invalidateQueries({ queryKey: questKeys.streaks });
      void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
      void balance.refresh();
      toast.success(
        t("streaks.restored", {
          name: streak.friend?.displayName ?? "",
          count: response.streak?.days ?? streak.lostDays,
        }),
      );
    },
    onError: (error) => {
      setRestoring(null);
      void queryClient.invalidateQueries({ queryKey: questKeys.streaks });
      if (errorReason(error) === "INSUFFICIENT_COINS")
        toast.error(t("errors.INSUFFICIENT_COINS"), {
          action: {
            label: t("streaks.buyCoins"),
            onClick: () => void navigate({ to: "/wallet/buy" }),
          },
        });
      else toast.error(errorMessage(error, { namespace: "quest" }));
    },
  });

  const pages = query.data?.pages ?? [];
  const offer = pages[0]?.restore;
  const streaks = pages.flatMap((page) => page.streaks);
  const rows = streaks.map((streak) => ({ streak, status: streakStatus(streak, now, zone) }));
  const lost = rows.filter((row) => row.status === "lost");
  const running = rows.filter((row) => row.status !== "lost" && row.status !== "none");
  const cost = offer?.costCoins ?? 0;
  const short = balance.coins !== undefined && balance.coins < BigInt(cost);

  return (
    <Page title={t("streaks.title")} back="/quest" actions={<BalancePill />}>
      <QuestNav />
      {query.isPending ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : query.isError ? (
        <EmptyState
          emoji="🛰️"
          title={t("streaks.error")}
          action={
            <Button variant="secondary" onClick={() => void query.refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : streaks.length === 0 ? (
        <EmptyState
          emoji="🔥"
          title={t("streaks.empty")}
          body={t("streaks.emptyBody")}
          action={
            <Link
              to="/chat"
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              <MessageCircle className="size-4" />
              {t("streaks.startChat")}
            </Link>
          }
        />
      ) : (
        <>
          {lost.length > 0 && (
            <section aria-label={t("streaks.lost")} className="mb-7">
              <SectionTitle>{t("streaks.lost")}</SectionTitle>
              <p className="mt-1 text-sm text-ink-soft">
                {offer?.available
                  ? t("streaks.restoreHint", { coins: cost })
                  : t("streaks.restoreUsed", {
                      date: offer?.availableAgainOn
                        ? formatDay(offer.availableAgainOn, i18n.language)
                        : "",
                    })}
              </p>
              <ul className="mt-3 flex flex-col gap-3">
                {lost.map(({ streak }) => (
                  <LostRow
                    key={streak.friend?.id}
                    streak={streak}
                    now={now}
                    cost={cost}
                    available={Boolean(offer?.available)}
                    onRestore={() => setRestoring(streak)}
                  />
                ))}
              </ul>
            </section>
          )}

          {running.length > 0 && (
            <section aria-label={t("streaks.running")}>
              <SectionTitle aside={t("streaks.count", { count: running.length })}>
                {t("streaks.running")}
              </SectionTitle>
              <ul className="mt-3 flex flex-col gap-3">
                {running.map(({ streak, status }) => (
                  <StreakRow key={streak.friend?.id} streak={streak} status={status} now={now} />
                ))}
              </ul>
            </section>
          )}
          <LoadMore query={query} />

          <p className="mt-7 rounded-2xl bg-surface-2 px-4 py-3 text-sm leading-relaxed text-ink-soft">
            {t("streaks.rules")}
          </p>
        </>
      )}

      <ConfirmDialog
        open={restoring !== null}
        onOpenChange={(open) => !open && setRestoring(null)}
        title={t("streaks.restoreConfirm.title", {
          name: restoring?.friend?.displayName ?? "",
          count: restoring?.lostDays ?? 0,
        })}
        description={t("streaks.restoreConfirm.body")}
        confirmLabel={t("streaks.restoreConfirm.action", { coins: cost })}
        loading={restore.isPending}
        onConfirm={() => restoring && restore.mutate(restoring)}
      >
        <dl className="flex flex-col gap-2 rounded-2xl bg-surface-2 p-4 text-sm font-bold">
          <div className="flex items-center justify-between">
            <dt>{t("streaks.restoreConfirm.cost")}</dt>
            <dd>
              <Coins amount={cost} />
            </dd>
          </div>
          <div className="flex items-center justify-between">
            <dt>{t("streaks.restoreConfirm.balance")}</dt>
            <dd>{balance.coins === undefined ? "…" : <Coins amount={balance.coins} />}</dd>
          </div>
        </dl>
        {short && (
          <p className="mt-3 text-sm font-bold text-danger">
            {t("streaks.restoreConfirm.short")}{" "}
            <Link to="/wallet/buy" className="underline">
              {t("streaks.buyCoins")}
            </Link>
          </p>
        )}
      </ConfirmDialog>
    </Page>
  );
}

const STATUS_TINT: Record<StreakStatus, string> = {
  none: "text-ink-faint",
  lost: "text-danger",
  expiring: "text-danger",
  mine: "text-brand",
  theirs: "text-ink-soft",
  both: "text-brand",
  done: "text-success",
};

function StreakRow({
  streak,
  status,
  now,
}: {
  streak: quest.Streak;
  status: StreakStatus;
  now: number;
}) {
  const { t } = useTranslation("quest");
  const left = useRemainingText();
  const friend = streak.friend;
  if (!friend) return null;
  const name = friend.displayName;
  const expiring = status === "expiring";
  const myMove = status === "mine" || status === "both" || expiring;
  const next = streak.nextMilestone;
  const previous = MILESTONES.filter((days) => days <= streak.days).at(-1) ?? 0;
  return (
    <li
      aria-label={t("streaks.rowLabel", { name, count: streak.days })}
      data-status={status}
      className={cn("sticker rounded-card p-4", expiring ? "bg-sun text-[#17122b]" : "bg-surface")}
    >
      <div className="flex items-center gap-3">
        <Link to="/u/$handle" params={{ handle: friend.handle }} className="shrink-0">
          <Avatar user={friend} size="md" outlined />
        </Link>
        <div className="min-w-0 flex-1">
          <p className="truncate font-black">
            <EmojiText text={name} />
          </p>
          <p className={cn("text-sm font-bold", expiring ? "text-[#17122b]" : STATUS_TINT[status])}>
            {expiring && streak.expiresAt
              ? t("streaks.status.expiring", { left: left(toMillis(streak.expiresAt), now) })
              : t(`streaks.status.${status}`, { name })}
          </p>
        </div>
        <p className="shrink-0 text-right leading-none">
          <span className="text-3xl font-black tabular-nums">
            <span aria-hidden>{expiring ? "⌛" : "🔥"}</span>
            {streak.days}
          </span>
          <span className="block pt-1 text-[11px] font-bold">{t("streaks.daysUnit")}</span>
        </p>
      </div>

      {next > 0 && (
        <div className="mt-3 flex items-center gap-2">
          <ProgressBar
            className={cn("h-2 flex-1 border-0", expiring ? "bg-surface" : "bg-surface-3")}
            value={fraction(streak.days - previous, next - previous)}
            label={t("streaks.milestoneBar", { name })}
            valueText={t("streaks.toMilestone", { count: next - streak.days, days: next })}
          />
          <span className="shrink-0 text-xs font-black tabular-nums">
            {t("streaks.toMilestone", { count: next - streak.days, days: next })}
          </span>
        </div>
      )}

      <div className="mt-3 flex items-center justify-between gap-3">
        <span className={cn("text-xs font-bold", !expiring && "text-ink-faint")}>
          {t("streaks.best", { count: streak.bestDays })}
        </span>
        <Button
          size="sm"
          variant={myMove ? "primary" : "secondary"}
          icon={<MessageCircle className="size-4" />}
          onClick={() => void openDirectChat(friend.id)}
        >
          {myMove ? t("streaks.keepGoing") : t("streaks.message")}
        </Button>
      </div>
    </li>
  );
}

function LostRow({
  streak,
  now,
  cost,
  available,
  onRestore,
}: {
  streak: quest.Streak;
  now: number;
  cost: number;
  available: boolean;
  onRestore(): void;
}) {
  const { t } = useTranslation("quest");
  const left = useRemainingText();
  const friend = streak.friend;
  if (!friend) return null;
  const until = streak.restorableUntil ? toMillis(streak.restorableUntil) : 0;
  return (
    <li
      aria-label={t("streaks.lostLabel", { name: friend.displayName, count: streak.lostDays })}
      className="sticker flex flex-wrap items-center gap-3 rounded-card bg-surface p-4"
    >
      <Avatar user={friend} size="md" outlined className="grayscale" />
      <div className="min-w-0 flex-1">
        <p className="truncate font-black">
          <EmojiText text={friend.displayName} />
        </p>
        <p className="text-sm font-bold text-danger">
          💔 {t("streaks.lostDays", { count: streak.lostDays })}
        </p>
        {until > now && (
          <p className="text-xs font-bold text-ink-faint">
            {t("streaks.restorableFor", { left: left(until, now) })}
          </p>
        )}
      </div>
      <Button variant="lime" size="sm" disabled={!available || until <= now} onClick={onRestore}>
        {t("streaks.restore")} <Coins amount={cost} size="sm" />
      </Button>
    </li>
  );
}
