import type { AccountLifecycle } from "#/seams/lifecycle";
import { closeViewings } from "./playback";

/**
 * A video being watched stops being reported: where the viewer stopped goes
 * out while the account's session still speaks for it, and nothing follows —
 * the player may go on (the small window, the lock screen) until the screen
 * closes under the next account.
 */
export default { beforeLeave: closeViewings } satisfies AccountLifecycle;
