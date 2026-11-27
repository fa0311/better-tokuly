import type { SignOutCleanup } from "#/seams/sign-out";
import { LEGACY_KEY, PREFIX } from "./seen-levels";

/** The account's levels this device already celebrated. */
export default { storage: [PREFIX, LEGACY_KEY] } satisfies SignOutCleanup;
