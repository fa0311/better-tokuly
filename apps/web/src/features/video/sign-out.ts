import type { SignOutCleanup } from "#/seams/sign-out";
import { forgetStudioChannel } from "./studio/shared";

/** The channel the account's video studio showed. */
export default { forget: forgetStudioChannel } satisfies SignOutCleanup;
