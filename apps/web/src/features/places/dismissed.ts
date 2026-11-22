import type { geo } from "@tokuly/api";
import { forgetDeviceMemory, opaqueHash, readDeviceMemory, writeDeviceMemory } from "./device-memory";

/**
 * Suggestions the person answered "not now". The server keeps suggesting a
 * spot until it is a place and has no way to be told otherwise, so the
 * answer is remembered on this device: per account, as fingerprints rather
 * than coordinates, and only for as long as the spot is still suggested.
 */
const PREFIX = "tokuly.places.dismissed.";
/** The device-wide list of coordinates an earlier version kept: removed wherever it is still found. */
const LEGACY_KEY = "tokuly.places.dismissedSuggestions";

/** The fingerprint of a suggestion for one account (the spot at about 100 m, mixed with the user ID). */
export function suggestionMark(userId: string, suggestion: geo.PlaceSuggestion): string {
  return opaqueHash(`${userId}|${suggestion.center?.lat.toFixed(3)},${suggestion.center?.lng.toFixed(3)}`);
}

export function readDismissed(userId: string): string[] {
  const stored = readDeviceMemory<unknown>(PREFIX + userId);
  return Array.isArray(stored) ? stored.filter((entry): entry is string => typeof entry === "string") : [];
}

/**
 * Remembers `suggestion` as put off. Marks of spots that are no longer
 * among `current` are dropped: once a spot stops being suggested there is
 * nothing left to remember about it.
 */
export function dismissSuggestion(userId: string, suggestion: geo.PlaceSuggestion, current: geo.PlaceSuggestion[]): string[] {
  const live = new Set(current.map((entry) => suggestionMark(userId, entry)));
  const next = [...new Set([...readDismissed(userId), suggestionMark(userId, suggestion)])].filter((mark) => live.has(mark));
  writeDeviceMemory(PREFIX + userId, next);
  return next;
}

/** Forgets every answer kept in this browser, for every account. */
export function forgetDismissed(): void {
  forgetDeviceMemory(PREFIX, LEGACY_KEY);
}
