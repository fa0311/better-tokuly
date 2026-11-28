import type { SignOutCleanup } from "#/seams/sign-out";
import { applyAccent, DEFAULT_ACCENT } from "./appearance/accent";

/** The signed-out pages wear the default colour again: the account's look was its own. */
export default { forget: () => applyAccent(DEFAULT_ACCENT) } satisfies SignOutCleanup;
