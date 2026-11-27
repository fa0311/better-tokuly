import type { BadgeSources } from "#/seams/badges";
import { useDailyBonus } from "./data";

/** Today's login bonus, while it waits to be claimed. */
export default { wallet: () => (useDailyBonus().data?.claimable ? 1 : 0) } satisfies BadgeSources;
