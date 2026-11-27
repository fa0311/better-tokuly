import type { SignOutCleanup } from "#/seams/sign-out";
import { closeStoryViewer, useStoryViewer } from "./viewer-store";

/** Nothing of the account keeps playing once its session is over. */
export default {
  forget() {
    if (useStoryViewer.getState().reels) closeStoryViewer();
  },
} satisfies SignOutCleanup;
