import type { AccountLifecycle } from "#/seams/lifecycle";
import { useLive } from "./live";
import { clearOutbox } from "./outbox";

/**
 * What the account had on its way (a message that failed waits with
 * "retry"), who was typing to it and how far its conversations were read
 * are not the next one's.
 */
export default {
  forget() {
    clearOutbox();
    useLive.getState().reset();
  },
} satisfies AccountLifecycle;
