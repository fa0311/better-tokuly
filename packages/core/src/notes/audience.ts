import { common } from "@tokuly/api";

/** Who a note is shared with: a visibility, the circle of a CIRCLE note, the people of a direct note. */
export interface Audience {
  visibility: common.Visibility;
  circleId?: string;
  /** A direct note (PRIVATE naming people). */
  directUserIds?: string[];
  /** Chosen as a direct note, even before anyone is picked. */
  direct?: boolean;
}

export type AudienceKey =
  | "public"
  | "followers"
  | "friends"
  | "closeFriends"
  | "circle"
  | "direct"
  | "private";

export interface AudienceOption {
  key: AudienceKey;
  value: common.Visibility;
  /** Widest first; a reply cannot pick a smaller rank than the note it answers. */
  rank: number;
}

/** Who a note can be shown to, widest first. */
export const AUDIENCE_OPTIONS: readonly AudienceOption[] = [
  { key: "public", value: common.Visibility.PUBLIC, rank: 0 },
  { key: "followers", value: common.Visibility.FOLLOWERS, rank: 1 },
  { key: "friends", value: common.Visibility.FRIENDS, rank: 2 },
  { key: "closeFriends", value: common.Visibility.CLOSE_FRIENDS, rank: 3 },
  { key: "circle", value: common.Visibility.CIRCLE, rank: 3 },
  { key: "direct", value: common.Visibility.PRIVATE, rank: 3 },
  { key: "private", value: common.Visibility.PRIVATE, rank: 4 },
];

function option(key: AudienceKey): AudienceOption {
  return AUDIENCE_OPTIONS.find((candidate) => candidate.key === key)!;
}

/** The option an audience is shown as. A value this app does not know reads as the narrowest, never as public. */
export function audienceOption(audience: Audience): AudienceOption {
  if (audience.visibility === common.Visibility.PRIVATE)
    return option(audience.direct || audience.directUserIds?.length ? "direct" : "private");
  return (
    AUDIENCE_OPTIONS.find((candidate) => candidate.value === audience.visibility) ??
    option("private")
  );
}

/** The audience a note has, as the viewer is told. */
export function noteAudience(note: {
  visibility: common.Visibility;
  circleId?: string;
  directRecipients?: { id: string }[];
}): Audience {
  return {
    visibility: note.visibility,
    circleId: note.circleId || undefined,
    directUserIds: note.directRecipients?.map((user) => user.id),
  };
}

export function visibilityOption(visibility: common.Visibility): AudienceOption {
  return audienceOption({ visibility });
}

export function sameAudience(a: Audience, b: Audience): boolean {
  const people = (audience: Audience) => [...(audience.directUserIds ?? [])].sort().join(",");
  return (
    a.visibility === b.visibility &&
    (a.circleId ?? "") === (b.circleId ?? "") &&
    people(a) === people(b)
  );
}

/**
 * The audience a new note is shown as: the one wanted, except that a reply
 * is never wider than the note it answers — and a reply to close friends, a
 * circle or named people goes back directly.
 */
export function replyAudience(wanted: Audience, replyTo: Audience | undefined): AudienceOption {
  const chosen = audienceOption(wanted);
  if (!replyTo) return chosen;
  const ceiling = audienceOption(replyTo);
  if (ceiling.rank <= chosen.rank) return chosen;
  return ceiling.rank >= 3 ? option("direct") : ceiling;
}

/** What CreateNote / SaveDraft take for an audience. `undefined` leaves it to the account's default. */
export function audienceFields(audience: Audience | undefined): {
  visibility: common.Visibility;
  circleId: string;
  directUserIds: string[];
} {
  return {
    visibility: audience?.visibility ?? common.Visibility.UNSPECIFIED,
    circleId: audience?.circleId ?? "",
    directUserIds: audience?.directUserIds ?? [],
  };
}
