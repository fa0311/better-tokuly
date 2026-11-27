import type { SignOutCleanup } from "#/seams/sign-out";
import { MOMENT_SEEN_PREFIX } from "./countdown/today-moment";
import { AUDIENCE_PREFIX } from "./free/model";

/** Whom the account last showed its free time to, and which of its moments it has seen. */
export default { storage: [AUDIENCE_PREFIX, MOMENT_SEEN_PREFIX] } satisfies SignOutCleanup;
