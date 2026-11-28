import { forgetUserStories } from "@tokuly/core/stories";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their reel leaves the story tray (a viewer playing it closes by itself). */
export default forgetUserStories satisfies ForgetUser;
