import { Link } from "@tanstack/react-router";
import { engagement, isApiError, toMillis } from "@tokuly/api";
import {
  compactCount,
  emojiReaction,
  isCustomEmoji,
  LIKE,
  type ReactionPicture,
  type Subject,
  sameReaction,
  toPicture,
} from "@tokuly/core/engagement";
import { Heart, SmilePlus, UsersRound } from "lucide-react";
import { type ReactNode, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { useEngagement, useReact, useReactors, useUnreact } from "./api";
import { useBurst } from "./burst";
import { EmojiPicker } from "./emoji-picker";
import { useSubjectLive } from "./live";
import { useReactionPops } from "./pops";
import { ReactionEmoji, reactionLabel } from "./reaction-emoji";

/** Chips beyond this many are folded into the "who reacted" button. */
const VISIBLE_CHIPS = 8;

/** What flies out of a control for an emoji: the emoji itself, sparkles for a custom one (a picture is no glyph). */
const burstGlyph = (emoji: string) => (isCustomEmoji(emoji) ? "✨" : emoji);

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
 * with — Unicode and custom, drawn with their pictures (tap one to join in,
 * tap yours to take it back), a picker to add another, and the list of who
 * reacted. Counts move live while it is on screen, and what others add pops
 * up over it. Renders nothing for a subject the viewer cannot see.
 */
export function ReactionBar({ subject, like = true, className }: ReactionBarProps) {
  const { data } = useEngagement(subject);
  useSubjectLive(subject);
  if (data === null) return null;
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {like && <LikeButton subject={subject} />}
      <ReactionChips subject={subject} className="contents" />
      {data?.rules.emoji !== false && <AddReactionButton subject={subject} />}
    </div>
  );
}

/** The like of a subject (not for notes, which have their own): a heart, its count, a burst when it lands. */
export function LikeButton({ subject, className }: { subject: Subject; className?: string }) {
  const { t, i18n } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const unreact = useUnreact(subject);
  const { fire, burst } = useBurst(["💜", "✨", "💖"]);
  const liked = Boolean(
    data?.viewerReactions.some((reaction) => reaction.kind === engagement.ReactionKind.LIKE),
  );
  const disabled = !data?.interactionsEnabled;

  return (
    <button
      type="button"
      aria-pressed={liked}
      aria-label={t("reactions.like")}
      disabled={disabled}
      onClick={() => {
        if (!liked) fire();
        // Only the like: on a subject that takes several reactions the emoji stay.
        void (liked ? unreact(LIKE) : react(LIKE));
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
 * reacts with that emoji (or takes the viewer's own back): on a subject that
 * takes one reaction per person it replaces theirs, on one that takes
 * several it joins them. An emoji that was taken down shows its shortcode
 * and can only be taken back. Renders nothing while nobody reacted, so it
 * can sit in a card without taking a row.
 */
export function ReactionChips({ subject, className }: { subject: Subject; className?: string }) {
  const { t } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const unreact = useUnreact(subject);
  const [listOpen, setListOpen] = useState(false);
  if (!data || data.emojiCounts.length === 0) return null;

  const mine = (emoji: string) =>
    data.viewerReactions.some((reaction) => sameReaction(reaction, emojiReaction(emoji)));
  const shown = data.emojiCounts.slice(0, VISIBLE_CHIPS);
  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {shown.map((entry) => {
        const held = mine(entry.emoji);
        const gone = entry.custom?.available === false;
        return (
          <ReactionChip
            key={entry.emoji}
            emoji={entry.emoji}
            custom={entry.custom}
            count={entry.count}
            mine={held}
            disabled={!data.interactionsEnabled || (gone && !held)}
            onToggle={() =>
              void (held ? unreact(emojiReaction(entry.emoji)) : react(emojiReaction(entry.emoji)))
            }
          />
        );
      })}
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
  custom,
  count,
  mine,
  disabled,
  onToggle,
}: {
  emoji: string;
  custom?: ReactionPicture;
  count: number;
  mine: boolean;
  disabled: boolean;
  onToggle(): void;
}) {
  const { t, i18n } = useTranslation("engagement");
  const { fire, burst } = useBurst([burstGlyph(emoji)]);
  return (
    <button
      type="button"
      aria-pressed={mine}
      aria-label={t("reactions.chip", { emoji: reactionLabel(emoji, custom), count })}
      data-emoji={emoji}
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
      <span aria-hidden className="inline-flex items-center text-base leading-none">
        <ReactionEmoji emoji={emoji} custom={custom} />
      </span>
      <span className="tabular-nums">{compactCount(count, i18n.language)}</span>
      {burst}
    </button>
  );
}

/**
 * The "add a reaction" button: opens the emoji picker — the person's deck,
 * recently used, custom and Unicode emoji (custom ones only where the
 * subject takes them) — and reacts with the pick. What others add pops up
 * out of it while it is on screen.
 */
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
  const pops = useReactionPops(subject.type, data?.subjectId);
  const disabled = !data?.interactionsEnabled;
  // A subject full of different emoji takes only the ones it carries (the server lists them all then).
  const full = Boolean(data && data.distinctEmoji >= data.rules.maxDistinctEmoji);
  const present = data?.emojiCounts
    .filter((entry) => entry.custom?.available !== false)
    .map((entry) => entry.emoji);
  return (
    <EmojiPicker
      custom={data?.rules.customEmoji ?? true}
      hideSensitive={!data?.rules.sensitiveEmoji}
      only={full ? present : undefined}
      onPick={(emoji) => {
        fire([burstGlyph(emoji), "✨"]);
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
          {pops}
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
              <span className="inline-flex items-center gap-1">
                <ReactionEmoji emoji={entry.emoji} custom={entry.custom} size={18} />
                {entry.count}
              </span>
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
                      <span className="block truncate font-bold">
                        <EmojiText text={reactor.user.displayName} />
                      </span>
                      <span className="block truncate text-sm text-ink-soft">
                        @{reactor.user.handle} · {compactAge(toMillis(reactor.reactedAt))}
                      </span>
                    </span>
                    <span
                      className="text-2xl leading-none"
                      role="img"
                      aria-label={
                        reactor.reaction?.kind === engagement.ReactionKind.EMOJI
                          ? reactionLabel(
                              reactor.reaction.emoji,
                              toPicture(reactor.reaction.custom),
                            )
                          : t("reactions.like")
                      }
                    >
                      {reactor.reaction?.kind === engagement.ReactionKind.EMOJI ? (
                        <ReactionEmoji
                          emoji={reactor.reaction.emoji}
                          custom={toPicture(reactor.reaction.custom)}
                          size={28}
                        />
                      ) : (
                        "💜"
                      )}
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
