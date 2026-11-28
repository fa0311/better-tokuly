import type { SignOutCleanup } from "#/seams/sign-out";
import { setBadge } from "#/stores/badges";
import { hangUp } from "./calls/session";
import { useLive } from "./live";
import { clearOutbox } from "./outbox";

/** Nothing of the account's chats greets the next person: messages on their way, a call, who types. */
export default {
  forget() {
    void hangUp();
    clearOutbox();
    useLive.getState().reset();
    setBadge("chat", 0);
  },
} satisfies SignOutCleanup;
