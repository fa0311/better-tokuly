export interface CreateSearch {
  /** Start with this sound ("use this sound"). */
  sound?: string;
  /** Make a remix or a duet of this short. */
  remix?: string;
  kind?: "remix" | "duet";
  /** Start from this clip instead of a file. */
  clip?: string;
  /** Start the caption with this hashtag. */
  tag?: string;
}

export function validateCreateSearch(search: Record<string, unknown>): CreateSearch {
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const remix = text(search.remix);
  return {
    ...(text(search.sound) ? { sound: text(search.sound) } : {}),
    ...(remix
      ? { remix, kind: search.kind === "duet" ? ("duet" as const) : ("remix" as const) }
      : {}),
    ...(text(search.clip) ? { clip: text(search.clip) } : {}),
    ...(text(search.tag) ? { tag: text(search.tag) } : {}),
  };
}
