import { type common, engagement } from "@tokuly/api";

/**
 * What every engagement control hangs off: the type of a piece of content and
 * its ID (the UUID, or the short ID for content that has one).
 */
export interface Subject {
  type: common.SubjectType;
  id: string;
}

export const subjectKey = (subject: Subject): string => `${subject.type}:${subject.id}`;

/** The realtime topic of a subject. It takes the UUID, never a short ID. */
export const subjectTopic = (type: common.SubjectType, uuid: string): string =>
  `subject:${type}:${uuid}`;

/** The caller's own reaction: a like, a dislike or one emoji. */
export interface ViewerReaction {
  kind: engagement.ReactionKind;
  /** Only for `ReactionKind.EMOJI`. */
  emoji: string;
}

export interface EmojiCount {
  emoji: string;
  count: number;
}

/** Every engagement number of a subject plus the caller's own state, as the controls read it. */
export interface Engagement {
  /** The subject's UUID: what realtime events name it by. */
  subjectId: string;
  likeCount: number;
  dislikeCount: number;
  /** The most used emoji first. */
  emojiCounts: EmojiCount[];
  emojiTotal: number;
  viewerReaction?: ViewerReaction;
  commentCount: number;
  bookmarkCount: number;
  shareCount: number;
  bookmarked: boolean;
  /** False when the subject takes no reactions right now. */
  interactionsEnabled: boolean;
}

export const LIKE: ViewerReaction = { kind: engagement.ReactionKind.LIKE, emoji: "" };

export const emojiReaction = (emoji: string): ViewerReaction => ({
  kind: engagement.ReactionKind.EMOJI,
  emoji,
});

export function sameReaction(
  a: ViewerReaction | undefined,
  b: ViewerReaction | undefined,
): boolean {
  return a?.kind === b?.kind && (a?.emoji ?? "") === (b?.emoji ?? "");
}

function toViewerReaction(reaction: engagement.Reaction | undefined): ViewerReaction | undefined {
  if (!reaction || reaction.kind === engagement.ReactionKind.UNSPECIFIED) return undefined;
  return { kind: reaction.kind, emoji: reaction.emoji };
}

/** The reaction part of an engagement, from what the server sent. */
export function reactionFields(
  summary: engagement.ReactionSummary | undefined,
): Pick<
  Engagement,
  "likeCount" | "dislikeCount" | "emojiCounts" | "emojiTotal" | "viewerReaction"
> {
  return {
    likeCount: Number(summary?.likeCount ?? 0n),
    dislikeCount: Number(summary?.dislikeCount ?? 0n),
    emojiCounts: (summary?.emojiCounts ?? []).map((entry) => ({
      emoji: entry.emoji,
      count: Number(entry.count),
    })),
    emojiTotal: Number(summary?.emojiTotal ?? 0n),
    viewerReaction: toViewerReaction(summary?.viewerReaction),
  };
}

export function toEngagement(message: engagement.SubjectEngagement): Engagement {
  return {
    subjectId: message.subject?.id ?? "",
    ...reactionFields(message.reactions),
    commentCount: Number(message.commentCount),
    bookmarkCount: Number(message.bookmarkCount),
    shareCount: Number(message.shareCount),
    bookmarked: message.bookmarked,
    interactionsEnabled: message.interactionsEnabled,
  };
}

function adjustEmoji(counts: EmojiCount[], emoji: string, delta: number): EmojiCount[] {
  const known = counts.some((entry) => entry.emoji === emoji);
  const next = known
    ? counts.map((entry) =>
        entry.emoji === emoji ? { ...entry, count: entry.count + delta } : entry,
      )
    : [...counts, { emoji, count: delta }];
  // The server lists the most used first; an emoji that ties keeps its place, so chips do not jump around under a finger.
  return next.filter((entry) => entry.count > 0);
}

/**
 * The engagement as it will be once the caller's reaction is `next`
 * (`undefined` = taken back): the previous reaction leaves its counter, the
 * new one joins its own. Used to show a tap at once, before the server answers.
 */
export function withViewerReaction(
  current: Engagement,
  next: ViewerReaction | undefined,
): Engagement {
  let { likeCount, dislikeCount, emojiCounts, emojiTotal } = current;
  const apply = (reaction: ViewerReaction | undefined, delta: number) => {
    if (!reaction) return;
    if (reaction.kind === engagement.ReactionKind.LIKE) likeCount = Math.max(0, likeCount + delta);
    else if (reaction.kind === engagement.ReactionKind.DISLIKE)
      dislikeCount = Math.max(0, dislikeCount + delta);
    else if (reaction.kind === engagement.ReactionKind.EMOJI) {
      emojiCounts = adjustEmoji(emojiCounts, reaction.emoji, delta);
      emojiTotal = Math.max(0, emojiTotal + delta);
    }
  };
  apply(current.viewerReaction, -1);
  apply(next, 1);
  return { ...current, likeCount, dislikeCount, emojiCounts, emojiTotal, viewerReaction: next };
}

/** A realtime `counters_changed` applied on top of what is cached: totals from the server, the caller's own state kept. */
export function withCounters(
  current: Engagement,
  counters: engagement.CountersChanged,
): Engagement {
  const { viewerReaction: _ignored, ...reactions } = reactionFields(counters.reactions);
  return {
    ...current,
    ...(counters.reactions ? reactions : {}),
    commentCount: Number(counters.commentCount),
    bookmarkCount: Number(counters.bookmarkCount),
    shareCount: Number(counters.shareCount),
  };
}

/** 1234 → "1,234", 12_345 → "1.2万" (ja) / "12K" (en): counts next to icons stay short. */
export function compactCount(count: number, language: string): string {
  if (count < 10_000) return count.toLocaleString(language);
  return new Intl.NumberFormat(language, { notation: "compact", maximumFractionDigits: 1 }).format(
    count,
  );
}
