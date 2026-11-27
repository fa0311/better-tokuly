import type { geo } from "@tokuly/api";
import { withDismissed } from "@tokuly/core/places";
import { forgetDeviceMemory, readDeviceMemory, writeDeviceMemory } from "#/lib/device-memory";

/**
 * Suggestions the person answered "not now". The server keeps suggesting a
 * spot until it is a place and has no way to be told otherwise, so the
 * answer is remembered on this device: per account, as fingerprints rather
 * than coordinates, and only for as long as the spot is still suggested.
 */
const PREFIX = "tokuly.places.dismissed.";
/** The device-wide list of coordinates an earlier version kept: removed wherever it is still found. */
const LEGACY_KEY = "tokuly.places.dismissedSuggestions";

export function readDismissed(userId: string): string[] {
  const stored = readDeviceMemory<unknown>(PREFIX + userId);
  return Array.isArray(stored)
    ? stored.filter((entry): entry is string => typeof entry === "string")
    : [];
}

/**
 * Remembers `suggestion` as put off. Marks of spots that are no longer
 * among `current` are dropped: once a spot stops being suggested there is
 * nothing left to remember about it.
 */
export function dismissSuggestion(
  userId: string,
  suggestion: geo.PlaceSuggestion,
  current: geo.PlaceSuggestion[],
): string[] {
  const next = withDismissed(userId, readDismissed(userId), suggestion, current);
  writeDeviceMemory(PREFIX + userId, next);
  return next;
}

/** Forgets every answer kept in this browser, for every account. */
export function forgetDismissed(): void {
  forgetDeviceMemory(PREFIX, LEGACY_KEY);
}
