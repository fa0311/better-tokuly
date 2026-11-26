import type { AccountLifecycle } from "#/seams/lifecycle";
import { stopWatching } from "./room";

/**
 * A stream being watched stops being reported as watched: the leaving
 * account's heartbeats end here, while the screen (and its player) may stay
 * until it closes under the next account.
 */
export default { beforeLeave: stopWatching } satisfies AccountLifecycle;
