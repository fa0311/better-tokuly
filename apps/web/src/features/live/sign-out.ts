import type { SignOutCleanup } from "#/seams/sign-out";
import { CHANNEL_KEY } from "./studio/shared";

/**
 * The channel the account's live studio showed. (An unanswered gift stays,
 * by account: whether its coins were spent is asked when that account is back.)
 */
export default { storage: [CHANNEL_KEY] } satisfies SignOutCleanup;
