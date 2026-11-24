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

/**
 * A custom emoji as reactions carry it: enough to draw it without the
 * catalogue. `available` is false once it left the catalogue (disabled,
 * deleted, renamed): show its shortcode as text; it can no longer be added.
 */
export interface ReactionPicture {
  /** The shortcode without colons. */
  name: string;
  /** The animated WebP of an animated emoji, the largest still otherwise. */
  url: string;
  /** Still renditions by longest edge, smallest first. */
  stills: { size: number; url: string }[];
  /** A still for less movement (the same as `url` for a still emoji). */
  staticUrl: string;
  animated: boolean;
  sensitive: boolean;
  available: boolean;
}

/** One reaction of the caller: a like, a dislike or one emoji (Unicode, or a custom `:name:`). */
export interface ViewerReaction {
  kind: engagement.ReactionKind;
  /** Only for `ReactionKind.EMOJI`. */
  emoji: string;
  /** For a custom emoji, as the server described it. */
  custom?: ReactionPicture;
}

export interface EmojiCount {
  emoji: string;
  count: number;
  custom?: ReactionPicture;
}

/** How a subject takes reactions. */
export interface ReactionRules {
  /** Emoji are taken at all (comments take likes and dislikes only). */
  emoji: boolean;
  /** One person may hold several distinct emoji; otherwise a new reaction replaces the old one. */
  multiple: boolean;
  maxPerPerson: number;
  maxDistinctEmoji: number;
  customEmoji: boolean;
  sensitiveEmoji: boolean;
}

/** What a subject takes until the server says otherwise: one reaction per person, Unicode and custom emoji. */
export const DEFAULT_RULES: ReactionRules = {
  emoji: true,
  multiple: false,
  maxPerPerson: 1,
  maxDistinctEmoji: 50,
  customEmoji: true,
  sensitiveEmoji: false,
};

/** Every engagement number of a subject plus the caller's own state, as the controls read it. */
export interface Engagement {
  /** The subject's UUID: what realtime events name it by. */
  subjectId: string;
  likeCount: number;
  dislikeCount: number;
  /** The most used emoji first. */
  emojiCounts: EmojiCount[];
  emojiTotal: number;
  /** How many different emoji the subject carries (all of them are in `emojiCounts` once it is full). */
  distinctEmoji: number;
  /** The caller's like or dislike if they gave one, otherwise their newest emoji. */
  viewerReaction?: ViewerReaction;
  /** Every reaction of the caller, oldest first. */
  viewerReactions: ViewerReaction[];
  rules: ReactionRules;
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

/** Whether an emoji (as reactions write it) is a custom one: `:name:`. */
export const isCustomEmoji = (emoji: string): boolean =>
  emoji.length > 2 && emoji.startsWith(":") && emoji.endsWith(":");

export function sameReaction(
  a: ViewerReaction | undefined,
  b: ViewerReaction | undefined,
): boolean {
  return a?.kind === b?.kind && (a?.emoji ?? "") === (b?.emoji ?? "");
}

/** A like or a dislike: a person holds at most one of the two. */
const isVote = (reaction: ViewerReaction): boolean =>
  reaction.kind !== engagement.ReactionKind.EMOJI;

/** The reaction that stands for all of a person's: their like or dislike, otherwise their newest emoji. */
export function representative(held: ViewerReaction[]): ViewerReaction | undefined {
  return held.find(isVote) ?? held.at(-1);
}

/** A custom emoji's description from the server. */
export function toPicture(
  custom: engagement.ReactionEmoji | undefined,
): ReactionPicture | undefined {
  if (!custom) return undefined;
  const stills = [...(custom.image?.variants ?? [])]
    .filter((variant) => variant.url)
    .map((variant) => ({ size: variant.size, url: variant.url }))
    .sort((a, b) => a.size - b.size);
  return {
    name: custom.name,
    url: custom.image?.url ?? "",
    stills,
    staticUrl: custom.staticUrl,
    animated: custom.animated,
    sensitive: custom.sensitive,
    available: custom.available,
  };
}

function toViewerReaction(reaction: engagement.Reaction | undefined): ViewerReaction | undefined {
  if (!reaction || reaction.kind === engagement.ReactionKind.UNSPECIFIED) return undefined;
  const custom = toPicture(reaction.custom);
  return custom
    ? { kind: reaction.kind, emoji: reaction.emoji, custom }
    : { kind: reaction.kind, emoji: reaction.emoji };
}

/** A reaction as the server sent it, for a row of a list. */
export const readReaction = toViewerReaction;

function toRules(rules: engagement.ReactionRules | undefined): ReactionRules {
  if (!rules) return DEFAULT_RULES;
  return {
    emoji: rules.emoji,
    multiple: rules.multiple,
    maxPerPerson: rules.maxPerPerson,
    maxDistinctEmoji: rules.maxDistinctEmoji,
    customEmoji: rules.customEmoji,
    sensitiveEmoji: rules.sensitiveEmoji,
  };
}

/** The counts of a summary (what a realtime push carries too). */
function countFields(
  summary: engagement.ReactionSummary | undefined,
): Pick<Engagement, "likeCount" | "dislikeCount" | "emojiCounts" | "emojiTotal" | "distinctEmoji"> {
  return {
    likeCount: Number(summary?.likeCount ?? 0n),
    dislikeCount: Number(summary?.dislikeCount ?? 0n),
    emojiCounts: (summary?.emojiCounts ?? []).map((entry) => {
      const custom = toPicture(entry.custom);
      return custom
        ? { emoji: entry.emoji, count: Number(entry.count), custom }
        : { emoji: entry.emoji, count: Number(entry.count) };
    }),
    emojiTotal: Number(summary?.emojiTotal ?? 0n),
    distinctEmoji: summary?.distinctEmoji ?? 0,
  };
}

/** The reaction part of an engagement, from what the server sent. */
export function reactionFields(
  summary: engagement.ReactionSummary | undefined,
): Pick<
  Engagement,
  | "likeCount"
  | "dislikeCount"
  | "emojiCounts"
  | "emojiTotal"
  | "distinctEmoji"
  | "viewerReaction"
  | "viewerReactions"
  | "rules"
> {
  const viewerReactions = (summary?.viewerReactions ?? [])
    .map(toViewerReaction)
    .filter((reaction): reaction is ViewerReaction => reaction !== undefined);
  const viewerReaction = toViewerReaction(summary?.viewerReaction);
  return {
    ...countFields(summary),
    viewerReaction,
    viewerReactions:
      viewerReactions.length > 0 || !viewerReaction ? viewerReactions : [viewerReaction],
    rules: toRules(summary?.rules),
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

function adjustEmoji(counts: EmojiCount[], reaction: ViewerReaction, delta: number): EmojiCount[] {
  const known = counts.some((entry) => entry.emoji === reaction.emoji);
  const next = known
    ? counts.map((entry) =>
        entry.emoji === reaction.emoji ? { ...entry, count: entry.count + delta } : entry,
      )
    : [
        ...counts,
        reaction.custom
          ? { emoji: reaction.emoji, count: delta, custom: reaction.custom }
          : { emoji: reaction.emoji, count: delta },
      ];
  // The server lists the most used first; an emoji that ties keeps its place, so chips do not jump around under a finger.
  return next.filter((entry) => entry.count > 0);
}

/** What the caller does with their reactions to one subject. */
export type ReactionChange =
  | { add: ViewerReaction }
  | { remove: ViewerReaction }
  | { removeAll: true };

/**
 * The engagement as it will be after `change`, under the subject's rules:
 * on a subject that takes one reaction a new one replaces the old; on one
 * that takes several, emoji sit side by side and a like and a dislike
 * replace each other. Used to show a tap at once, before the server answers.
 */
export function withReactionChange(current: Engagement, change: ReactionChange): Engagement {
  const held = current.viewerReactions;
  let removed: ViewerReaction[];
  let added: ViewerReaction | undefined;
  if ("removeAll" in change) removed = held;
  else if ("remove" in change) removed = held.filter((mine) => sameReaction(mine, change.remove));
  else {
    if (held.some((mine) => sameReaction(mine, change.add))) return current;
    added = change.add;
    removed = current.rules.multiple ? (isVote(added) ? held.filter(isVote) : []) : held;
  }
  let { likeCount, dislikeCount, emojiCounts, emojiTotal, distinctEmoji } = current;
  const apply = (reaction: ViewerReaction, delta: number) => {
    if (reaction.kind === engagement.ReactionKind.LIKE) likeCount = Math.max(0, likeCount + delta);
    else if (reaction.kind === engagement.ReactionKind.DISLIKE)
      dislikeCount = Math.max(0, dislikeCount + delta);
    else if (reaction.kind === engagement.ReactionKind.EMOJI) {
      const before = emojiCounts.length;
      emojiCounts = adjustEmoji(emojiCounts, reaction, delta);
      distinctEmoji = Math.max(0, distinctEmoji + emojiCounts.length - before);
      emojiTotal = Math.max(0, emojiTotal + delta);
    }
  };
  for (const reaction of removed) apply(reaction, -1);
  if (added) apply(added, 1);
  const viewerReactions = [
    ...held.filter((mine) => !removed.some((gone) => sameReaction(gone, mine))),
    ...(added ? [added] : []),
  ];
  return {
    ...current,
    likeCount,
    dislikeCount,
    emojiCounts,
    emojiTotal,
    distinctEmoji,
    viewerReactions,
    viewerReaction: representative(viewerReactions),
  };
}

/**
 * The engagement as it will be once the caller's reaction is `next`
 * (`undefined` = every reaction taken back).
 */
export function withViewerReaction(
  current: Engagement,
  next: ViewerReaction | undefined,
): Engagement {
  return withReactionChange(current, next ? { add: next } : { removeAll: true });
}

/** A realtime `counters_changed` applied on top of what is cached: totals from the server, the caller's own state and the rules kept. */
export function withCounters(
  current: Engagement,
  counters: engagement.CountersChanged,
): Engagement {
  return {
    ...current,
    ...(counters.reactions ? countFields(counters.reactions) : {}),
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
