import type { SignOutCleanup } from "#/seams/sign-out";
import { resetActiveSession } from "./session-store";

/** The study timer belongs to the account: it does not outlive the session. */
export default { forget: resetActiveSession } satisfies SignOutCleanup;
