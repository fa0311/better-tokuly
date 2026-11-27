import type { SignOutCleanup } from "#/seams/sign-out";
import { SENSITIVE_KEY, setShowSensitive } from "./prefs";

/** Sensitive emoji are hidden again: showing them was the account's choice. */
export default {
  storage: [SENSITIVE_KEY],
  forget: () => setShowSensitive(false),
} satisfies SignOutCleanup;
