import type { SignOutCleanup } from "#/seams/sign-out";
import { IDENTITY_KEY } from "./composer";

/** Which of the account's channels it last commented as. */
export default { storage: [IDENTITY_KEY] } satisfies SignOutCleanup;
