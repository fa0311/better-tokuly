import type { SignOutCleanup } from "#/seams/sign-out";
import { LEGACY_KEY, PREFIX } from "./dismissed";

/** What this device remembered about the account's suggestions. */
export default { storage: [PREFIX, LEGACY_KEY] } satisfies SignOutCleanup;
