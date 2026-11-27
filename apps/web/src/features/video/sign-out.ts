import type { SignOutCleanup } from "#/seams/sign-out";
import { CHANNEL_KEY, forgetStudioChannel } from "./studio/shared";

/** The channel the account's video studio showed. */
export default { storage: [CHANNEL_KEY], forget: forgetStudioChannel } satisfies SignOutCleanup;
