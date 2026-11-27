import type { AccountLifecycle } from "#/seams/lifecycle";
import { closeViewings } from "./playback";
import { closeSaveSheet } from "./save-sheet";

/**
 * A video being watched stops being reported: where the viewer stopped goes
 * out while the account's session still speaks for it, and nothing follows —
 * the player may go on (the small window, the lock screen) until the screen
 * closes under the next account. A "save to…" sheet it opened does not open
 * for the next one.
 */
export default { beforeLeave: closeViewings, forget: closeSaveSheet } satisfies AccountLifecycle;
