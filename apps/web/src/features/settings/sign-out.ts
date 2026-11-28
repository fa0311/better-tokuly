import type { SignOutCleanup } from "#/seams/sign-out";
import { applyColors, DEFAULT_COLORS } from "./appearance/colors";
import { OWNER_KEY } from "./background";
import { REQUEST_PREFIX } from "./your-data/export-card";

/**
 * The browser's claim on the account's look ends, so the next sign-in starts
 * from the account again, and the signed-out pages wear the default colours.
 * The export the account asked for is not "on its way" for the next person.
 */
export default {
  storage: [OWNER_KEY, REQUEST_PREFIX],
  forget: () => applyColors(DEFAULT_COLORS),
} satisfies SignOutCleanup;
