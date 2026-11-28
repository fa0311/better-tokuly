import type { SignOutCleanup } from "#/seams/sign-out";
import { resetDeletedNotes } from "./api";
import { closeCompose } from "./compose-store";
import { resetViews } from "./views";

/** The composer, and what this tab noted about views and deleted notes. */
export default {
  forget() {
    closeCompose();
    resetViews();
    resetDeletedNotes();
  },
} satisfies SignOutCleanup;
