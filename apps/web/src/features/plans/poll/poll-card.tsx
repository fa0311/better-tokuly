import { Link } from "@tanstack/react-router";
import { plans, toDate } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { nameOf } from "../api";
import { formatDay, formatTime, ymd } from "../dates";
import { slotText } from "./model";

export function PollStatusChip({ poll }: { poll: plans.Poll }) {
  const { t } = useTranslation("plans");
  if (poll.status === plans.PollStatus.FINALIZED)
    return <Chip className="bg-lime text-[#17122b]">{t("poll.status.finalized")}</Chip>;
  if (poll.status === plans.PollStatus.CLOSED) return <Chip>{t("poll.status.closed")}</Chip>;
  return <Chip className="bg-sky text-[#17122b]">{t("poll.status.open")}</Chip>;
}

/** A date poll in a list: how far along it is and whether the caller still has to answer. */
export function PollCard({ poll }: { poll: plans.Poll }) {
  const { t, i18n } = useTranslation("plans");
  const open = poll.status === plans.PollStatus.OPEN;
  const best = poll.slots.find((slot) => slot.id === (poll.finalSlotId || poll.bestSlotId));
  const deadline = toDate(poll.deadline);
  return (
    <Link
      to="/plans/polls/$pollId"
      params={{ pollId: poll.shortId || poll.id }}
      className={cn(
        "pressable sticker flex gap-3 rounded-card bg-surface p-4",
        !open && "opacity-80",
      )}
    >
      <span
        className="grid size-14 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-3xl"
        aria-hidden
      >
        🗳️
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-lg font-black">{poll.title}</span>
          {open && !poll.answeredByMe ? (
            <Chip className="bg-pink text-white">{t("poll.needsAnswer")}</Chip>
          ) : (
            <PollStatusChip poll={poll} />
          )}
        </span>
        <span className="mt-0.5 block truncate text-sm font-bold text-ink-soft">
          {t("poll.cardMeta", {
            slots: poll.slots.length,
            answered: poll.answeredCount,
            total: poll.participantCount,
          })}
          {!poll.mine && ` · ${nameOf(poll.creator)}`}
        </span>
        {best && (
          <span className="mt-1 block truncate text-sm font-bold">
            {poll.finalSlotId ? "🎉" : "👑"} {slotText(best, i18n.language)}
          </span>
        )}
        {open && deadline && (
          <span className="mt-0.5 block truncate text-xs font-bold text-ink-faint">
            {t("poll.deadline", {
              time: `${formatDay(ymd(deadline), i18n.language)} ${formatTime(deadline, i18n.language)}`,
            })}
          </span>
        )}
      </span>
    </Link>
  );
}
