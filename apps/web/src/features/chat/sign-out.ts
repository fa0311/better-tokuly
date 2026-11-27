import type { SignOutCleanup } from "#/seams/sign-out";
import { setBadge } from "#/stores/badges";
import { CONSENT_KEY } from "./assist-consent";
import { hangUp } from "./calls/session";
import { DRAFTS_PREFIX } from "./drafts";
import { useLive } from "./live";
import { clearOutbox } from "./outbox";

/** Nothing of the account's chats greets the next person: drafts, messages on their way, a call, who types. */
export default {
  storage: [DRAFTS_PREFIX, CONSENT_KEY],
  forget() {
    void hangUp();
    clearOutbox();
    useLive.getState().reset();
    setBadge("chat", 0);
  },
} satisfies SignOutCleanup;
