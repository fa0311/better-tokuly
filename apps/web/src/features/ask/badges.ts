import type { BadgeSource } from "#/seams/badges";
import { useAskBadge } from "./api";

/** Unread questions plus unseen flames. */
export default (() => useAskBadge()) satisfies BadgeSource;
