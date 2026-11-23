import type { BadgeSources } from "#/seams/badges";
import { useMissions } from "./api";
import { claimable } from "./model";

/** The quest counter: missions whose reward waits to be collected. Realtime events keep the list current. */
export default {
  quest: () => claimable(useMissions().data).length,
} satisfies BadgeSources;
