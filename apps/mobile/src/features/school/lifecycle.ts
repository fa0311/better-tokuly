import type { AccountLifecycle } from "#/seams/lifecycle";
import { stopAs, stopTimer } from "./session";

/**
 * The study timer ends with the account that started it: it is logged with
 * that account's own API, and its reminder is taken back.
 */
export default {
  beforeLeave: stopAs,
  beforeEnd: stopTimer,
} satisfies AccountLifecycle;
