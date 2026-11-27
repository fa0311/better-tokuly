import type { geo } from "@tokuly/api";

/**
 * Suggestions the person answered "not now". The server keeps suggesting a
 * spot until it is a place and has no way to be told otherwise, so each app
 * remembers the answer on the device: per account, as fingerprints rather
 * than coordinates, and only for as long as the spot is still suggested.
 */

/**
 * A short, one-way fingerprint of a text (FNV-1a, 32 bits, base 36). Short
 * on purpose: many different inputs share each value, so a stored hash
 * cannot be turned back into the place it came from, while two things the
 * same account remembers practically never collide.
 */
export function opaqueHash(text: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

/** The fingerprint of a suggestion for one account (the spot at about 100 m, mixed with the user ID). */
export function suggestionMark(userId: string, suggestion: geo.PlaceSuggestion): string {
  return opaqueHash(
    `${userId}|${suggestion.center?.lat.toFixed(3)},${suggestion.center?.lng.toFixed(3)}`,
  );
}

/**
 * The marks to keep after putting `suggestion` off. Marks of spots that are
 * no longer among `current` are dropped: once a spot stops being suggested
 * there is nothing left to remember about it.
 */
export function withDismissed(
  userId: string,
  marks: readonly string[],
  suggestion: geo.PlaceSuggestion,
  current: readonly geo.PlaceSuggestion[],
): string[] {
  const live = new Set(current.map((entry) => suggestionMark(userId, entry)));
  return [...new Set([...marks, suggestionMark(userId, suggestion)])].filter((mark) =>
    live.has(mark),
  );
}

/** The suggestions still to show: with a spot, and not put off. */
export function shownSuggestions(
  userId: string,
  suggestions: readonly geo.PlaceSuggestion[],
  dismissed: readonly string[],
): geo.PlaceSuggestion[] {
  return suggestions.filter(
    (suggestion) => suggestion.center && !dismissed.includes(suggestionMark(userId, suggestion)),
  );
}
