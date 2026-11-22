import { Link } from "@tanstack/react-router";
import { engagement, isApiError, toMillis } from "@tokuly/api";
import { Heart, SmilePlus, UsersRound } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useEngagement, useReact, useReactors } from "./api";
import { useBurst } from "./burst";
import { EmojiPicker } from "./emoji-picker";
import { useSubjectLive } from "./live";
import { compactCount, emojiReaction, LIKE, type Subject } from "./model";

/** Chips beyond this many are folded into the "who reacted" button. */
const VISIBLE_CHIPS = 8;

export interface ReactionBarProps {
  subject: Subject;
  /**
   * Show the like button. Content with likes of its own (notes) turns it
   * off and keeps the emoji reactions.
   */
  like?: boolean;
  className?: string;
}

/**
 * Reactions of any subject: a like with its count, the emoji people reacted
 * with (tap one to join in, tap yours to take it back), a picker to add
 * another, and the list of who reacted. Counts move live while it is on
 * screen. Renders nothing for a subject the viewer cannot see.
 */
export function ReactionBar({ subject, like = true, className }: ReactionBarProps) {
  const { data } = useEngagement(subject);
  useSubjectLive(subject);
  if (data === null) return null;
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {like && <LikeButton subject={subject} />}
      <ReactionChips subject={subject} className="contents" />
      <AddReactionButton subject={subject} />
    </div>
  );
}

/** The like of a subject (not for notes, which have their own): a heart, its count, a burst when it lands. */
export function LikeButton({ subject, className }: { subject: Subject; className?: string }) {
  const { t, i18n } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const { fire, burst } = useBurst(["💜", "✨", "💖"]);
  const liked = data?.viewerReaction?.kind === engagement.ReactionKind.LIKE;
  const disabled = !data?.interactionsEnabled;

  return (
    <button
      type="button"
      aria-pressed={liked}
      aria-label={t("reactions.like")}
      disabled={disabled}
      onClick={() => {
        if (!liked) fire();
        void react(liked ? undefined : LIKE);
      }}
      className={cn(
        "relative inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-black transition-colors disabled:opacity-50",
        liked ? "bg-pink/15 text-pink" : "text-ink-soft hover:bg-surface-2 hover:text-pink",
        className,
      )}
    >
      <Heart
        className={cn(
          "size-[18px] transition-transform duration-200 ease-bounce",
          liked && "scale-110 fill-current",
        )}
      />
      {data && data.likeCount > 0 && (
        <span className="tabular-nums">{compactCount(data.likeCount, i18n.language)}</span>
      )}
      {burst}
    </button>
  );
}

/**
 * The emoji people reacted with, most used first, each with its count. A tap
 * reacts with that emoji (or takes the viewer's own back). Renders nothing
 * while nobody reacted, so it can sit in a card without taking a row.
 */
export function ReactionChips({ subject, className }: { subject: Subject; className?: string }) {
  const { t } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const [listOpen, setListOpen] = useState(false);
  if (!data || data.emojiCounts.length === 0) return null;

  const mine =
    data.viewerReaction?.kind === engagement.ReactionKind.EMOJI ? data.viewerReaction.emoji : "";
  const shown = data.emojiCounts.slice(0, VISIBLE_CHIPS);
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {shown.map((entry) => (
        <ReactionChip
          key={entry.emoji}
          emoji={entry.emoji}
          count={entry.count}
          mine={entry.emoji === mine}
          disabled={!data.interactionsEnabled}
          onToggle={() => void react(entry.emoji === mine ? undefined : emojiReaction(entry.emoji))}
        />
      ))}
      <button
        type="button"
        aria-label={t("reactions.who")}
        onClick={() => setListOpen(true)}
        className="inline-flex h-8 items-center gap-1 rounded-full px-2 text-xs font-black text-ink-faint hover:bg-surface-2 hover:text-ink"
      >
        <UsersRound className="size-4" />
        {data.emojiTotal > shown.reduce((sum, entry) => sum + entry.count, 0) && (
          <span className="tabular-nums">{data.emojiTotal}</span>
        )}
      </button>
      {listOpen && <ReactorsDialog subject={subject} onClose={() => setListOpen(false)} />}
    </div>
  );
}

function ReactionChip({
  emoji,
  count,
  mine,
  disabled,
  onToggle,
}: {
  emoji: string;
  count: number;
  mine: boolean;
  disabled: boolean;
  onToggle(): void;
}) {
  const { t, i18n } = useTranslation("engagement");
  const { fire, burst } = useBurst([emoji]);
  return (
    <button
      type="button"
      aria-pressed={mine}
      aria-label={t("reactions.chip", { emoji, count })}
      disabled={disabled}
      onClick={() => {
        if (!mine) fire();
        onToggle();
      }}
      className={cn(
        "relative inline-flex h-8 animate-pop-in items-center gap-1 rounded-full border-2 px-2.5 text-sm font-black transition-colors disabled:opacity-60",
        mine
          ? "border-brand bg-brand-soft text-ink"
          : "border-line bg-surface text-ink-soft hover:border-ink-faint",
      )}
    >
      <span aria-hidden className="text-base leading-none">
        {emoji}
      </span>
      <span className="tabular-nums">{compactCount(count, i18n.language)}</span>
      {burst}
    </button>
  );
}

/** The "add a reaction" button: opens the emoji picker and reacts with the pick. */
export function AddReactionButton({
  subject,
  className,
}: {
  subject: Subject;
  className?: string;
}) {
  const { t } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const { fire, burst } = useBurst(["✨", "🎉", "💫"]);
  const disabled = !data?.interactionsEnabled;
  return (
    <EmojiPicker
      onPick={(emoji) => {
        fire([emoji, "✨"]);
        void react(emojiReaction(emoji));
      }}
      trigger={
        <button
          type="button"
          aria-label={t("reactions.add")}
          disabled={disabled}
          className={cn(
            "relative grid size-9 place-items-center rounded-full text-ink-soft transition-colors hover:bg-surface-2 hover:text-brand disabled:opacity-50",
            className,
          )}
        >
          <SmilePlus className="size-[18px]" />
          {burst}
        </button>
      }
    />
  );
}

/** Who reacted, most recent first, with a filter per emoji. Who liked is only listed for the subject's owner. */
export function ReactorsDialog({ subject, onClose }: { subject: Subject; onClose(): void }) {
  const { t } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const [emoji, setEmoji] = useState("");
  const query = useReactors(subject, emoji, true);
  const reactors = query.data?.pages.flatMap((page) => page.reactors) ?? [];
  const isPrivate = isApiError(query.error);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("reactions.whoTitle")}>
      {data && data.emojiCounts.length > 1 && (
        <div className="scrollbar-none -mx-1 mb-3 flex gap-1.5 overflow-x-auto px-1 pb-1">
          <FilterChip active={emoji === ""} onClick={() => setEmoji("")}>
            {t("reactions.all")}
          </FilterChip>
          {data.emojiCounts.map((entry) => (
            <FilterChip
              key={entry.emoji}
              active={emoji === entry.emoji}
              onClick={() => setEmoji(entry.emoji)}
            >
              {entry.emoji} {entry.count}
            </FilterChip>
          ))}
        </div>
      )}
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : reactors.length === 0 ? (
        <EmptyState
          emoji={isPrivate ? "🙈" : "🫧"}
          title={isPrivate ? t("reactions.listPrivate") : t("reactions.nobody")}
        />
      ) : (
        <ul className="flex flex-col">
          {reactors.map(
            (reactor) =>
              reactor.user && (
                <li key={`${reactor.user.id}:${reactor.reaction?.emoji ?? ""}`}>
                  <Link
                    to="/u/$handle"
                    params={{ handle: reactor.user.handle }}
                    onClick={onClose}
                    className="flex items-center gap-3 rounded-2xl px-2 py-2 hover:bg-surface-2"
                  >
                    <Avatar user={reactor.user} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold">{reactor.user.displayName}</span>
                      <span className="block truncate text-sm text-ink-soft">
                        @{reactor.user.handle} · {compactAge(toMillis(reactor.reactedAt))}
                      </span>
                    </span>
                    <span
                      className="text-2xl leading-none"
                      role="img"
                      aria-label={reactor.reaction?.emoji || t("reactions.like")}
                    >
                      {reactor.reaction?.kind === engagement.ReactionKind.EMOJI
                        ? reactor.reaction.emoji
                        : "💜"}
                    </span>
                  </Link>
                </li>
              ),
          )}
          <LoadMore query={query} />
        </ul>
      )}
    </Dialog>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "h-8 shrink-0 rounded-full border-2 px-3 text-sm font-black",
        active
          ? "border-ink bg-ink text-surface"
          : "border-line text-ink-soft hover:border-ink-faint",
      )}
    >
      {children}
    </button>
  );
}
