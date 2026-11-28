import type { SignOutCleanup } from "#/seams/sign-out";
import { applyColors, DEFAULT_COLORS } from "./appearance/colors";

/**
 * The signed-out pages wear the default colours again: the account's colours
 * were its own (their stored copy, `tokuly.colors`, goes with the account's
 * other storage, and they come back from the server at the next sign-in).
 */
export default { forget: () => applyColors(DEFAULT_COLORS) } satisfies SignOutCleanup;
