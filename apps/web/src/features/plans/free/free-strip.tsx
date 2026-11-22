import { Link } from "@tanstack/react-router";
import type { plans } from "@tokuly/api";
import { Check, ChevronRight, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { nameOf, useFreeBoard } from "../api";
import type { PlanDraft } from "../draft";
import { useNow } from "../parts";
import { useAnswerFree } from "./answer";
import { FreeFriendSheet } from "./friend-sheet";
import { badgeText, expiryText, hasEnded, isRunning } from "./model";
import { SetFreeSheet } from "./set-free-sheet";

export interface FreeFriendsStripProps {
  /** Leave the heading (and the link to the ひま page) out, for places that have their own. */
  bare?: boolean;
  className?: string;
}

/**
 * Who is free right now, as a strip of faces that scrolls sideways: the
 * person's own tile first ("ひま!" sets a status), then every friend whose
 * status they may see, each with an "あそぼ!" button that answers in one tap.
 * It keeps itself current and drops statuses the moment they run out.
 */
export function FreeFriendsStrip({ bare, className }: FreeFriendsStripProps) {
  const { t } = useTranslation("plans");
  const me = useMe();
  const now = useNow(15_000);
  const board = useFreeBoard();
  const answer = useAnswerFree();
  const [setting, setSetting] = useState(false);
  const [opened, setOpened] = useState<{ userId: string; draft?: PlanDraft }>();

  const statuses = (board.data?.statuses ?? []).filter(
    (status) => status.user && !hasEnded(status, now),
  );
  const mine = board.data?.mine && !hasEnded(board.data.mine, now) ? board.data.mine : undefined;
  const openedStatus = opened && statuses.find((status) => status.user?.id === opened.userId);

  const asobo = (status: plans.FreeStatus) => {
    const userId = status.user?.id ?? "";
    if (status.answeredByMe) return setOpened({ userId });
    // One tap answers; the sheet that follows offers a chat or a plan.
    answer.mutate(
      { status },
      { onSuccess: (response) => setOpened({ userId, draft: response.planDraft }) },
    );
  };

  return (
    <section aria-label={t("free.strip.title")} className={className}>
      {!bare && (
        <div className="mb-2 flex items-center gap-2 px-1">
          <h2 className="min-w-0 flex-1 truncate text-base font-black">
            {t("free.strip.title")}
            {statuses.length > 0 && (
              <span className="ml-2 rounded-full bg-lime px-2 py-0.5 text-xs font-black text-[#17122b]">
                {statuses.length}
              </span>
            )}
          </h2>
          <Link
            to="/plans/free"
            className="flex items-center text-sm font-bold text-ink-soft hover:text-ink"
          >
            {t("free.strip.all")}
            <ChevronRight className="size-4" />
          </Link>
        </div>
      )}
      <ul className="scrollbar-none -mx-4 flex snap-x gap-3 overflow-x-auto px-4 pt-1 pb-2">
        <li className="snap-start">
          {mine ? (
            <Link
              to="/plans/free"
              className="pressable sticker flex h-44 w-32 flex-col items-center gap-1 rounded-card bg-lime p-3 text-center text-[#17122b]"
            >
              <PulseFace user={me?.user} live={isRunning(mine, now)} />
              <span className="mt-1 text-sm font-black">{t("free.strip.mine")}</span>
              <span className="line-clamp-2 text-xs font-bold opacity-80">
                {mine.note || expiryText(mine, now, t)}
              </span>
              {mine.answerCount > 0 && (
                <span className="mt-auto rounded-full bg-pink px-2 py-0.5 text-xs font-black text-white">
                  {t("free.strip.answers", { count: mine.answerCount })}
                </span>
              )}
            </Link>
          ) : (
            <button
              type="button"
              onClick={() => setSetting(true)}
              className="pressable sticker flex h-44 w-32 flex-col items-center justify-center gap-2 rounded-card bg-surface p-3 text-center"
            >
              <span className="grid size-14 place-items-center rounded-[1.1rem] border-2 border-dashed border-ink-faint text-ink-soft">
                <Plus className="size-7" />
              </span>
              <span className="text-lg font-black">{t("free.strip.set")}</span>
              <span className="text-xs font-bold text-ink-soft">{t("free.strip.setHint")}</span>
            </button>
          )}
        </li>
        {board.isPending &&
          [0, 1].map((i) => (
            <li key={i}>
              <Skeleton className="h-44 w-32 rounded-card" />
            </li>
          ))}
        {statuses.map((status) => (
          <li key={status.user?.id} className="snap-start">
            <FriendTile
              status={status}
              now={now}
              onOpen={() => setOpened({ userId: status.user?.id ?? "" })}
              onAsobo={() => asobo(status)}
            />
          </li>
        ))}
        {board.paging.hasNextPage && (
          <li className="flex h-44 w-16 shrink-0 items-center justify-center">
            <LoadMore query={board.paging} />
          </li>
        )}
        {!board.isPending && statuses.length === 0 && !board.paging.hasNextPage && (
          <li className="flex h-44 w-56 shrink-0 flex-col justify-center rounded-card border-2 border-dashed border-line px-4 text-sm font-bold text-ink-soft">
            <span className="text-2xl">🦗</span>
            <span className="mt-1">{t("free.strip.empty")}</span>
            <span className="text-xs text-ink-faint">{t("free.strip.emptyBody")}</span>
          </li>
        )}
      </ul>

      <SetFreeSheet open={setting} onOpenChange={setSetting} current={mine} />
      <FreeFriendSheet
        status={openedStatus}
        draft={opened?.draft}
        onOpenChange={(open) => !open && setOpened(undefined)}
      />
    </section>
  );
}

function FriendTile({
  status,
  now,
  onOpen,
  onAsobo,
}: {
  status: plans.FreeStatus;
  now: number;
  onOpen(): void;
  onAsobo(): void;
}) {
  const { t, i18n } = useTranslation("plans");
  const live = isRunning(status, now);
  return (
    <div
      className="sticker flex h-44 w-32 flex-col items-center rounded-card bg-surface p-3 text-center"
      data-testid="free-friend"
    >
      <button
        type="button"
        onClick={onOpen}
        className="flex min-h-0 w-full flex-1 flex-col items-center gap-0.5"
        aria-label={t("free.friend.title", { name: nameOf(status.user) })}
      >
        <PulseFace user={status.user} live={live} />
        <span className="mt-1 w-full truncate text-sm font-black">
          <EmojiText text={nameOf(status.user)} />
        </span>
        <span
          className={cn(
            "max-w-full truncate rounded-full px-2 text-[11px] font-black",
            live ? "bg-lime text-[#17122b]" : "bg-surface-2 text-ink-soft",
          )}
        >
          {badgeText(status, now, i18n.language, t)}
        </span>
        {status.note && (
          <span className="line-clamp-1 w-full text-xs font-bold text-ink-soft">{status.note}</span>
        )}
      </button>
      <button
        type="button"
        onClick={onAsobo}
        className={cn(
          "pressable sticker-sm mt-1.5 inline-flex h-8 w-full shrink-0 items-center justify-center gap-1 rounded-full text-sm font-black",
          status.answeredByMe ? "bg-surface-2 text-ink-soft" : "bg-pink text-white",
        )}
      >
        {status.answeredByMe ? (
          <>
            <Check className="size-4" strokeWidth={3} />
            {t("free.asoboSent")}
          </>
        ) : (
          t("free.asobo")
        )}
      </button>
    </div>
  );
}

/** A face with a ring that pulses while the person is free right now. */
export function PulseFace({ user, live }: { user: plans.FreeStatus["user"]; live: boolean }) {
  return (
    <span className="relative grid place-items-center">
      {live && (
        <span
          className="absolute inset-0 animate-pulse-ring rounded-[1.1rem] bg-lime"
          aria-hidden
        />
      )}
      <Avatar
        user={user}
        size="lg"
        outlined
        className="relative [&>span]:size-14 [&>span]:rounded-[1.1rem]"
      />
    </span>
  );
}
