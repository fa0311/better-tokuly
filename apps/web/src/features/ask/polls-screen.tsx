import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ask, type common, errorReason, toMillis } from "@tokuly/api";
import { Flame, Shuffle, SkipForward } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { useBurst } from "#/features/engagement";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { askKeys, complimentClient } from "./api";
import { countdownClock, localized } from "./model";
import { askError, toastError } from "./parts";
import { serverNow } from "./server-clock";

/** Tile colours of the four choices, in order. */
const TINTS = ["bg-sky", "bg-pink", "bg-lime", "bg-sun"] as const;

/** How long the pick is celebrated before the next poll comes in, at ドパガキ度 3. */
const PICK_HOLD_MS = 650;

/** After the server answered "still cooling down" to a start, the clock stays up at least this long before start is offered again. */
const STILL_COOLING_MS = 3000;

/** The server's time, ticking once a second until `until` has passed (for a countdown on screen). */
function useServerNow(until: number): number {
  const [now, setNow] = useState(() => serverNow());
  useEffect(() => {
    setNow(serverNow());
    if (until <= serverNow()) return;
    const timer = setInterval(() => {
      const current = serverNow();
      setNow(current);
      if (current >= until) clearInterval(timer);
    }, 1000);
    return () => clearInterval(timer);
  }, [until]);
  return now;
}

/**
 * Compliment polls (ほめ投票): a round of kind questions, each answered by
 * picking one of four friends, who receives an anonymous flame. Between
 * rounds: the reward for the one just finished and the time until the next.
 */
export function PollsScreen() {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: askKeys.round, queryFn: async () => (await complimentClient().getRound({})).round!, staleTime: 0, gcTime: 0 });
  // Set when this visit finished a round: the reward is celebrated once.
  const [finishedNow, setFinishedNow] = useState(false);
  // Server time until which the countdown stays, whatever this device believes: set when a start was answered with "not yet".
  const [coolingUntil, setCoolingUntil] = useState(0);
  const start = async () => {
    setFinishedNow(false);
    const answer = await query.refetch();
    if (answer.data?.state === ask.RoundState.COOLDOWN) setCoolingUntil(Math.max(toMillis(answer.data.nextRoundAt), serverNow() + STILL_COOLING_MS));
  };
  const round = query.data;
  const reason = query.isError ? errorReason(query.error) : undefined;

  const apply = (next: ask.Round | undefined) => {
    if (!next) return;
    if (next.state === ask.RoundState.COOLDOWN && round?.state === ask.RoundState.ACTIVE) {
      setFinishedNow(true);
      void queryClient.invalidateQueries({ queryKey: askKeys.topFlames });
    }
    queryClient.setQueryData(askKeys.round, next);
  };

  return (
    <Page
      title={t("polls.title")}
      back="/ask"
      actions={
        <Link to="/ask/flames" aria-label={t("polls.seeFlames")} className="grid size-9 place-items-center rounded-full hover:bg-surface-2">
          <Flame className="size-5" />
        </Link>
      }
    >
      {query.isPending ? (
        <div>
          <Skeleton className="h-40" />
          <div className="mt-4 grid grid-cols-2 gap-3">
            {TINTS.map((tint) => (
              <Skeleton key={tint} className="h-36" />
            ))}
          </div>
        </div>
      ) : reason === "NOT_ENOUGH_FRIENDS" ? (
        <EmptyState
          emoji="🫂"
          title={t("polls.notEnough")}
          body={t("polls.notEnoughBody")}
          action={
            <Link to="/friends" className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("polls.findFriends")}
            </Link>
          }
        />
      ) : query.isError || !round ? (
        <EmptyState
          emoji="😴"
          title={t("polls.unavailable")}
          body={reason === "POLLS_UNAVAILABLE" ? t("polls.unavailableBody") : askError(query.error)}
          action={
            <Button variant="secondary" onClick={() => query.refetch()}>
              {t("actions.retry", { ns: "common" })}
            </Button>
          }
        />
      ) : round.state === ask.RoundState.ACTIVE && round.poll ? (
        <PollView key={round.poll.id} round={round} poll={round.poll} onRound={apply} onStale={() => query.refetch()} />
      ) : (
        <Cooldown round={round} celebrate={finishedNow} coolingUntil={coolingUntil} onStart={start} starting={query.isFetching} />
      )}
    </Page>
  );
}

function PollView({ round, poll, onRound, onStale }: { round: ask.Round; poll: ask.Poll; onRound(round: ask.Round | undefined): void; onStale(): void }) {
  const { t, i18n } = useTranslation("ask");
  const [picked, setPicked] = useState<string | null>(null);
  const [busy, setBusy] = useState<"shuffle" | "skip" | null>(null);
  const locked = picked !== null || busy !== null;

  const fail = (error: unknown) => {
    toastError(error);
    // The round moved on somewhere else (another tab, another device): show where it is now.
    onStale();
  };
  const vote = async (user: common.UserSummary) => {
    if (locked) return;
    setPicked(user.id);
    const hold = new Promise((resolve) => setTimeout(resolve, PICK_HOLD_MS * dopamineFactor()));
    try {
      const [response] = await Promise.all([complimentClient().vote({ pollId: poll.id, userId: user.id }), hold]);
      toast.success(t("polls.sent", { name: user.displayName }), { id: "ask-poll-sent", duration: 1800 });
      onRound(response.round);
    } catch (error) {
      setPicked(null);
      fail(error);
    }
  };
  const act = async (kind: "shuffle" | "skip") => {
    if (locked) return;
    setBusy(kind);
    try {
      const response = kind === "shuffle" ? await complimentClient().shufflePoll({ pollId: poll.id }) : await complimentClient().skipPoll({ pollId: poll.id });
      onRound(response.round);
    } catch (error) {
      // Everyone who could be shown is on screen already: nothing went wrong.
      if (kind === "shuffle" && errorReason(error) === "NOT_ENOUGH_FRIENDS") toast(t("polls.noOthers"));
      else fail(error);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mx-auto max-w-md">
      <div className="flex items-center gap-3">
        <div className="h-3 flex-1 overflow-hidden rounded-full border-2 border-ink bg-surface" role="progressbar" aria-valuemin={0} aria-valuemax={round.pollCount} aria-valuenow={round.answeredCount}>
          <div className="h-full bg-lime transition-[width] duration-300" style={{ width: `${(round.answeredCount / Math.max(1, round.pollCount)) * 100}%` }} />
        </div>
        <span className="text-sm font-black tabular-nums" data-testid="poll-progress">
          {t("polls.progress", { current: poll.position, total: round.pollCount })}
        </span>
      </div>

      <div className="sticker mt-4 animate-pop-in rounded-card bg-brand px-5 py-5 text-center sm:py-7 text-on-brand" data-testid="poll-question">
        <div className="animate-float text-6xl">{poll.question?.emoji || "🔥"}</div>
        <p className="mt-3 text-2xl leading-snug font-black">{localized(poll.question?.text, i18n.language)}</p>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3" data-testid="poll-choices">
        {poll.choices.map((user, index) => (
          <ChoiceTile key={user.id} user={user} tint={TINTS[index % TINTS.length]!} state={picked === null ? "idle" : picked === user.id ? "picked" : "passed"} disabled={locked} onPick={() => vote(user)} />
        ))}
      </div>

      <div className="mt-5 flex items-center justify-center gap-3">
        <Button variant="secondary" icon={<Shuffle className="size-4" />} loading={busy === "shuffle"} disabled={locked || poll.shufflesLeft <= 0} onClick={() => act("shuffle")}>
          {t("polls.shuffle")}
          <span className="text-xs font-bold text-ink-soft">{t("polls.shufflesLeft", { count: poll.shufflesLeft })}</span>
        </Button>
        <Button variant="ghost" icon={<SkipForward className="size-4" />} loading={busy === "skip"} disabled={locked} onClick={() => act("skip")}>
          {t("polls.skip")}
        </Button>
      </div>
      <p className="mt-4 text-center text-xs font-bold text-ink-faint">{t("polls.secret")}</p>
    </div>
  );
}

/** One of the four friends. Picking it pops the tile and throws flames; how much follows the ドパガキ度. */
function ChoiceTile({ user, tint, state, disabled, onPick }: { user: common.UserSummary; tint: string; state: "idle" | "picked" | "passed"; disabled: boolean; onPick(): void }) {
  const { t } = useTranslation("ask");
  const { fire, burst } = useBurst(["🔥", "✨", "🔥", "💖"]);
  return (
    <span className="relative">
      <button
        type="button"
        aria-label={t("polls.pick", { name: user.displayName })}
        disabled={disabled}
        onClick={() => {
          fire();
          onPick();
        }}
        className={cn(
          "pressable sticker flex h-32 w-full flex-col sm:h-36 items-center justify-center gap-2 rounded-card px-2 text-[#17122b] disabled:cursor-default",
          tint,
          "transition-[transform,opacity,filter] duration-[calc(260ms*var(--dopa))] ease-bounce",
          state === "picked" && "z-10 scale-[1.08] -rotate-2",
          state === "passed" && "scale-95 opacity-40 saturate-50",
        )}
      >
        <Avatar user={user} size="lg" outlined />
        <span className="max-w-full truncate px-1 font-black">{user.displayName}</span>
        {state === "picked" && (
          <span aria-hidden className="absolute -top-3 -right-2 animate-pop-in text-4xl">
            🔥
          </span>
        )}
      </button>
      {burst}
    </span>
  );
}

/** Between rounds: what the finished one paid, and the clock until the next. */
function Cooldown({ round, celebrate, coolingUntil, onStart, starting }: { round: ask.Round; celebrate: boolean; /** Server time before which start is not offered. */ coolingUntil: number; onStart(): void; starting: boolean }) {
  const { t } = useTranslation("ask");
  // Counted against the server's clock: the round opens when the server says so, not when this device thinks it is time.
  const opensAt = Math.max(toMillis(round.nextRoundAt), coolingUntil);
  const left = opensAt - useServerNow(opensAt);
  const reward = round.rewardCoins;
  const { fire, burst } = useBurst(["🎉", "✨", "🪙", "🔥"]);
  // Once, when the round was finished on this visit.
  useEffect(() => {
    if (celebrate) fire();
  }, [celebrate]);

  return (
    <div className="mx-auto max-w-md text-center" data-testid="poll-cooldown">
      <div className="sticker relative animate-pop-in rounded-card bg-surface px-5 py-8">
        <div className="relative mx-auto w-fit text-6xl">
          {celebrate ? "🎉" : "⏳"}
          {burst}
        </div>
        {celebrate && <p className="mt-3 text-2xl font-black">{t("polls.done")}</p>}
        {celebrate &&
          (reward > 0n ? (
            <div className="mt-3 inline-flex items-center gap-2 rounded-full bg-lime px-4 py-2 text-[#17122b] sticker-sm" data-testid="poll-reward">
              <span className="text-sm font-black">{t("polls.doneReward")}</span>
              <Coins amount={reward} signed />
            </div>
          ) : (
            <p className="mt-2 text-sm font-bold text-ink-soft">{t("polls.doneNoReward")}</p>
          ))}
        {left > 0 ? (
          <>
            <p className="mt-5 text-sm font-bold text-ink-soft">{t("polls.next")}</p>
            <p className="mt-1 text-5xl font-black tabular-nums" role="timer" data-testid="poll-countdown">
              {countdownClock(left)}
            </p>
          </>
        ) : (
          <>
            <p className="mt-5 text-lg font-black">{t("polls.ready")}</p>
            <Button className="mt-3" size="lg" variant="lime" loading={starting} onClick={onStart}>
              {t("polls.start")}
            </Button>
          </>
        )}
      </div>
      <Link to="/ask/flames" className="pressable sticker mt-5 inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold">
        <Flame className="size-4" />
        {t("polls.seeFlames")}
      </Link>
    </div>
  );
}
