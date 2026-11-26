import type { BadgeSources } from "#/seams/badges";
import { useIncomingRequests } from "./data";

/** Requests waiting for an answer. */
export default {
  friends: () => useIncomingRequests().data?.requests.length ?? 0,
} satisfies BadgeSources;
