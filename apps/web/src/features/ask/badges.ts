import type { BadgeSources } from "#/seams/badges";
import { useAskBadge } from "./profile-ask";

/** `ask`: unread questions plus unseen flames. */
export default { ask: () => useAskBadge() } satisfies BadgeSources;
