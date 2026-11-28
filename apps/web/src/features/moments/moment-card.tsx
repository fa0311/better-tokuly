import { Link } from "@tanstack/react-router";
import { type moments, toMillis } from "@tokuly/api";
import { latenessText, reactionBlocker } from "@tokuly/core/moments";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { MomentPair } from "./pair";
import { ReactionPicker, ReactionStrip } from "./reactions";

/**
 * A moment in a list: who, how late, the two pictures, the caption and the
 * reactions. Locked, it says how to unlock it: take one's own moment
 * (`canTake`), or wait for the prompt.
 */
export function MomentCard({ moment, canTake }: { moment: moments.Moment; canTake: boolean }) {
  const { t, i18n } = useTranslation("moments");
  const author = moment.author;
  const late = Number(moment.lateSeconds) > 0;
  return (
    <article
      className="sticker rounded-card bg-surface p-3"
      aria-label={t("card.by", { name: author?.displayName ?? "" })}
      data-testid="moment-card"
    >
      <header className="mb-2.5 flex items-center gap-2.5">
        <Avatar user={author} size="sm" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-black">
            <EmojiText text={author?.displayName ?? ""} />
          </p>
          <p className="text-[11px] font-bold text-ink-soft">
            {compactAge(toMillis(moment.postedAt))}
          </p>
        </div>
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[11px] font-black",
            late ? "bg-surface-2 text-ink-soft" : "bg-lime text-[#17122b]",
          )}
          data-testid="moment-lateness"
        >
          {latenessText(i18n, moment.lateSeconds)}
        </span>
      </header>
      {moment.locked ? (
        <MomentPair
          moment={moment}
          locked={
            <>
              <p className="text-sm font-black drop-shadow">{t("locked.title")}</p>
              {canTake ? (
                <Link
                  to="/moments/camera"
                  className="pressable sticker rounded-full bg-lime px-4 py-2 text-sm font-black text-[#17122b]"
                >
                  {t("locked.action")}
                </Link>
              ) : (
                <p className="text-xs font-bold drop-shadow">{t("locked.waiting")}</p>
              )}
            </>
          }
        />
      ) : (
        <MomentPair moment={moment} />
      )}
      {!moment.locked && (
        <div className="mt-2.5 flex flex-col gap-2.5">
          {moment.caption && (
            <p className="text-[15px] leading-snug font-bold">
              <EmojiText text={moment.caption} />
            </p>
          )}
          <div className="flex items-center justify-between gap-2">
            <ReactionStrip moment={moment} />
            <Link
              to="/moments/$momentId"
              params={{ momentId: moment.id }}
              className="ml-auto text-xs font-black text-brand"
            >
              {t("card.seeAll")} →
            </Link>
          </div>
          {reactionBlocker(moment) === null && <ReactionPicker moment={moment} />}
          {moment.retakes > 0 && (
            <p className="text-[11px] font-bold text-ink-faint">
              {t("card.retakes", { count: moment.retakes })}
            </p>
          )}
        </div>
      )}
    </article>
  );
}
