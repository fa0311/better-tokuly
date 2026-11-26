import type { BadgeSources } from "#/seams/badges";
import { useUnread } from "./data";

export default { chat: () => useUnread().badge } satisfies BadgeSources;
