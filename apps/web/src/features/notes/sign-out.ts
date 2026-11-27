import type { SignOutCleanup } from "#/seams/sign-out";
import { resetDeletedNotes } from "./api";
import { closeCompose } from "./compose-store";
import { DRAFTS_PREFIX } from "./drafts";
import { resetViews } from "./views";

/** The drafts, the composer, and what this tab noted about views and deleted notes. */
export default {
  storage: [DRAFTS_PREFIX],
  forget() {
    closeCompose();
    resetViews();
    resetDeletedNotes();
  },
} satisfies SignOutCleanup;
