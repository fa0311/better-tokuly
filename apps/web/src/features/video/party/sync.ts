/** The authoritative playback of a party, in plain numbers. */
export interface SharedPlayback {
  playing: boolean;
  /** Position at `updatedAtMs`. */
  positionSeconds: number;
  rate: number;
  /** Server time the state was set. */
  updatedAtMs: number;
}

/** What a member's own player is doing. */
export interface LocalPlayback {
  positionSeconds: number;
  paused: boolean;
  rate: number;
}

/** What to do to a player so that it shows what the party is watching. */
export interface SyncAction {
  /** Jump here before anything else. */
  seekTo?: number;
  play?: boolean;
  pause?: boolean;
  rate?: number;
}

/** While playing, a player this far off the party is pulled back in line. */
export const DRIFT_TOLERANCE_SECONDS = 1.25;
/** While paused, everyone should sit on the same frame, give or take. */
const PAUSED_TOLERANCE_SECONDS = 0.4;

/** Where the party's video is at `serverNowMs`: a paused state stays put, a playing one moves on at its rate. */
export function expectedPosition(
  shared: SharedPlayback,
  serverNowMs: number,
  durationSeconds: number,
): number {
  const elapsed = shared.playing
    ? (Math.max(0, serverNowMs - shared.updatedAtMs) / 1000) * (shared.rate || 1)
    : 0;
  const position = shared.positionSeconds + elapsed;
  return durationSeconds > 0 ? Math.min(position, durationSeconds) : position;
}

/**
 * Compares a member's player with the party and says what has to change —
 * nothing at all when it is already in step, so calling this over and over
 * is harmless.
 */
export function syncAction(
  shared: SharedPlayback,
  local: LocalPlayback,
  serverNowMs: number,
  durationSeconds: number,
): SyncAction {
  const action: SyncAction = {};
  const target = expectedPosition(shared, serverNowMs, durationSeconds);
  const off = Math.abs(local.positionSeconds - target);
  const rate = shared.rate || 1;
  if (Math.abs(local.rate - rate) > 0.001) action.rate = rate;

  // A party that played to the end rests there; do not start a player that has nothing left to play.
  const atEnd = durationSeconds > 0 && target >= durationSeconds - 0.25;
  if (shared.playing && !atEnd) {
    if (off > DRIFT_TOLERANCE_SECONDS) action.seekTo = target;
    if (local.paused) action.play = true;
  } else {
    if (!local.paused) action.pause = true;
    if (off > PAUSED_TOLERANCE_SECONDS) action.seekTo = target;
  }
  return action;
}

/**
 * Whether something a controlling member's player just did is news for the
 * party. A player that merely followed the party (the seek, play or pause
 * that `syncAction` asked for) reports nothing; a member's own play, pause,
 * seek or speed change does.
 */
export function differsFromShared(
  shared: SharedPlayback,
  local: LocalPlayback,
  serverNowMs: number,
  durationSeconds: number,
): boolean {
  if (shared.playing === local.paused) return true;
  if (Math.abs(local.rate - (shared.rate || 1)) > 0.001) return true;
  return (
    Math.abs(local.positionSeconds - expectedPosition(shared, serverNowMs, durationSeconds)) >
    DRIFT_TOLERANCE_SECONDS
  );
}
