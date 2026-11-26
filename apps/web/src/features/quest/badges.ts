import { claimable } from "@tokuly/core/quest";
import type { BadgeSources } from "#/seams/badges";
import { useMissions } from "./api";

/** The quest counter: missions whose reward waits to be collected. Realtime events keep the list current. */
export default {
  quest: () => claimable(useMissions().data).length,
} satisfies BadgeSources;
