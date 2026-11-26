import type { BadgeSources } from "#/seams/badges";
import { useUnread } from "./data";

export default { notifications: () => useUnread().total } satisfies BadgeSources;
