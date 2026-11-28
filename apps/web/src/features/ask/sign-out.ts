import type { SignOutCleanup } from "#/seams/sign-out";
import { forgetPendingAsk } from "./guest-return";

/** A question a visitor wrote in this tab is theirs alone: when the account in use signs out, it goes. */
export default { forget: forgetPendingAsk } satisfies SignOutCleanup;
