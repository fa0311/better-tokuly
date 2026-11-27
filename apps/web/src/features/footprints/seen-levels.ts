import type { SeenLevels } from "@tokuly/core/footprints";
import { opaqueHash } from "@tokuly/core/places";
import { forgetDeviceMemory, readDeviceMemory, writeDeviceMemory } from "#/lib/device-memory";

/**
 * The levels that were already celebrated, so a level-up is announced
 * once. Kept on this device per account; regions are fingerprints, not
 * their IDs, so the entry does not say where the account has been; and it
 * is removed on sign-out.
 */
const PREFIX = "tokuly.footprints.levels.";
/** The device-wide entry with region IDs an earlier version kept: removed wherever it is still found. */
const LEGACY_KEY = "tokuly.footprints.levels";

/** What a region is remembered as for one account. */
export const regionMark = (userId: string, regionId: string): string =>
  opaqueHash(`${userId}|${regionId}`);

export function readSeenLevels(userId: string): SeenLevels | null {
  const seen = readDeviceMemory<SeenLevels>(PREFIX + userId);
  return seen &&
    typeof seen.explorer === "number" &&
    seen.regions &&
    typeof seen.regions === "object"
    ? seen
    : null;
}

export function writeSeenLevels(userId: string, seen: SeenLevels): void {
  writeDeviceMemory(PREFIX + userId, seen);
}

/** Forgets the celebrated levels of every account in this browser. */
export function forgetSeenLevels(): void {
  // The legacy key is a prefix of the per-account ones, so it is removed by exact name.
  forgetDeviceMemory(PREFIX);
  try {
    window.localStorage.removeItem(LEGACY_KEY);
  } catch {
    // Nothing to clear.
  }
}
