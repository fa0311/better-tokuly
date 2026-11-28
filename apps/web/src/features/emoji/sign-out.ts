import type { SignOutCleanup } from "#/seams/sign-out";
import { setShowSensitive } from "./prefs";

/** Sensitive emoji are hidden again: showing them was the account's choice. */
export default { forget: () => setShowSensitive(false) } satisfies SignOutCleanup;
