import type { AccountLifecycle } from "#/seams/lifecycle";
import { PENDING_GIFTS_KEY } from "./pending-gift";
import { stopWatching } from "./room";

/**
 * A stream being watched stops being reported as watched: the leaving
 * account's heartbeats end here, while the screen (and its player) may stay
 * until it closes under the next account. Its unanswered gifts go with the
 * accounts when a reinstalled app first starts.
 */
export default {
  beforeLeave: stopWatching,
  secureKeys: [PENDING_GIFTS_KEY],
} satisfies AccountLifecycle;
