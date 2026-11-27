import type { AccountLifecycle } from "#/seams/lifecycle";
import { ASKED_KEY } from "./reminders";
import { stopAs } from "./session";

/**
 * The study timer ends with the account that started it: it is logged with
 * that account's own API, and its reminder is taken back.
 *
 * No `beforeEnd`: a session that left offline is resumed whenever the phone
 * is next online, and the server has long stopped the timer it left (cut
 * back to its last heartbeat). A timer the account has by then was started
 * on another device.
 *
 * "Asked on this device" goes with the install: the iOS keychain outlives an
 * uninstall, the OS's own answer does not.
 */
export default {
  beforeLeave: stopAs,
  secureKeys: [ASKED_KEY],
} satisfies AccountLifecycle;
