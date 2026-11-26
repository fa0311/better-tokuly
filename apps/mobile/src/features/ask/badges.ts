import type { BadgeSources } from "#/seams/badges";
import { useInboxCounts, useUnseenFlames } from "./data";

/** `ask`: unread questions, plus flames not looked at yet (their list is on the web). */
export default {
  ask: () => (useInboxCounts().data?.unread ?? 0) + useUnseenFlames(),
} satisfies BadgeSources;
