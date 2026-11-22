import { Link } from "@tanstack/react-router";
import type { common, identity } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { useOpenTargetPath } from "#/features/notifications";
import { cn } from "#/lib/cn";
import { nameOf, useFreeBoard } from "../api";
import { daysBetween, formatDay, todayYmd } from "../dates";
import { useAnswerFree } from "../free/answer";
import { FreeFriendSheet } from "../free/friend-sheet";
import { expiryText, hasEnded, whenText } from "../free/model";
import { useNow } from "../parts";
import { countdownParts, localized, tintOf } from "./model";

export interface ProfilePlansProps {
  /** `Profile.cards` of the profile on show; cards of other domains are ignored. */
  cards: identity.ProfileCard[];
  /** Whose profile it is. */
  user: common.UserSummary;
  isSelf: boolean;
  className?: string;
}

/**
 * What this area adds to a profile: "ひま" while the person is free (with
 * "あそぼ!" for their friends), the countdowns they pinned, and a couple
 * counter both partners chose to show. Renders nothing when there is none.
 */
export function ProfilePlans({ cards, user, isSelf, className }: ProfilePlansProps) {
  const { t, i18n } = useTranslation("plans");
  const openPath = useOpenTargetPath();
  const free = cards.find((card) => card.kind === "plans.free");
  const countdowns = cards.filter((card) => card.kind === "plans.countdown");
  const anniversary = cards.find((card) => card.kind === "plans.anniversary");
  if (!free && countdowns.length === 0 && !anniversary) return null;

  return (
    <div className={cn("flex flex-col gap-3", className)} data-testid="profile-plans">
      {free && <FreeCard card={free} user={user} isSelf={isSelf} />}
      {anniversary && (
        <button type="button" onClick={() => openPath(anniversary.path)} className="pressable sticker flex items-center gap-3 rounded-card bg-pink p-4 text-left text-white" data-testid="profile-anniversary">
          <span className="text-3xl" aria-hidden>
            {anniversary.emoji || "💞"}
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate font-black">{localized(anniversary.title, i18n.language) || t("anniv.couple")}</span>
            <span className="block truncate text-sm font-bold opacity-85">{localized(anniversary.text, i18n.language)}</span>
          </span>
          {anniversary.data.days && (
            <span className="flex shrink-0 items-baseline gap-1 font-black">
              <span className="text-3xl leading-none tabular-nums">{anniversary.data.days}</span>
              <span className="text-sm">{t("anniv.daysUnit")}</span>
            </span>
          )}
        </button>
      )}
      {countdowns.length > 0 && (
        <ul className={cn("grid gap-3", countdowns.length > 1 ? "grid-cols-2 sm:grid-cols-3" : "grid-cols-1")}>
          {countdowns.map((card) => {
            const id = card.data.countdown_id ?? "";
            // Counted here, so the number is right for whoever is looking, today.
            const daysLeft = card.data.date ? daysBetween(todayYmd(), card.data.date) : Number(card.data.days_left ?? 0);
            const parts = countdownParts(daysLeft, t);
            return (
              <li key={id}>
                <Link to="/plans/countdowns/$countdownId" params={{ countdownId: id }} className={cn("pressable sticker flex h-full items-center gap-3 rounded-card p-3.5", tintOf(id))} data-testid="profile-countdown">
                  <span className="text-3xl" aria-hidden>
                    {card.emoji || "⏳"}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-black">{localized(card.title, i18n.language)}</span>
                    <span className="flex items-baseline gap-1 font-black">
                      {parts.lead && <span className="text-xs">{parts.lead}</span>}
                      <span className="text-2xl leading-none tabular-nums">{parts.number}</span>
                      {parts.unit && <span className="text-xs">{parts.unit}</span>}
                    </span>
                    {card.data.date && <span className="block truncate text-xs font-bold opacity-75">{formatDay(card.data.date, i18n.language)}</span>}
                  </span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/** "ひま" on a profile: the note and the time from the card, "あそぼ!" through the friend's live status. */
function FreeCard({ card, user, isSelf }: { card: identity.ProfileCard; user: common.UserSummary; isSelf: boolean }) {
  const { t, i18n } = useTranslation("plans");
  const now = useNow();
  const board = useFreeBoard();
  const answer = useAnswerFree();
  const [open, setOpen] = useState(false);
  const status = isSelf ? board.data?.mine : board.data?.statuses.find((other) => other.user?.id === user.id);
  const live = status && !hasEnded(status, now) ? status : undefined;
  // The card is as old as the profile; once the status itself is known to be over, it goes.
  if (board.data && !live) return null;
  const note = live?.note ?? card.data.note ?? "";
  const areaName = live?.areaName ?? card.data.area_name ?? "";

  return (
    <div className="sticker flex items-center gap-3 rounded-card bg-lime p-4 text-[#17122b]" data-testid="profile-free">
      <span className="relative grid shrink-0 place-items-center">
        <span className="absolute inset-0 animate-pulse-ring rounded-[0.85rem] bg-white" aria-hidden />
        <Avatar user={user} outlined className="relative" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-black">
          {isSelf ? t("free.mine.now") : t("free.profile", { name: nameOf(user) })}
          {areaName && <span className="ml-1.5 text-sm font-bold opacity-75">📍{areaName}</span>}
        </p>
        {note && <p className="truncate text-sm font-bold">「{note}」</p>}
        {live && (
          <p className="truncate text-xs font-bold opacity-75">
            {whenText(live, now, i18n.language, t)} · {expiryText(live, now, t)}
          </p>
        )}
      </div>
      {isSelf ? (
        <Link to="/plans/free" className="pressable sticker-sm inline-flex h-9 shrink-0 items-center rounded-full bg-surface px-4 text-sm font-black text-ink">
          {t("free.mine.edit")}
        </Link>
      ) : (
        live && (
          <Button variant="secondary" size="sm" className="shrink-0" loading={answer.isPending} onClick={() => (live.answeredByMe ? setOpen(true) : answer.mutate({ status: live }, { onSuccess: () => setOpen(true) }))}>
            {live.answeredByMe ? `✓ ${t("free.asoboSent")}` : t("free.asobo")}
          </Button>
        )
      )}
      {!isSelf && <FreeFriendSheet status={open ? live : undefined} onOpenChange={setOpen} />}
    </div>
  );
}
