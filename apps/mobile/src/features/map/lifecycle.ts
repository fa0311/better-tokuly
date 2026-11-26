import type { AccountLifecycle } from "#/seams/lifecycle";
import { leaveSharing } from "./sharing";

/** Nothing more is sent for the account, and the server hears that it stopped sharing from here. */
export default {
  beforeLeave: ({ userId }) => leaveSharing(userId),
} satisfies AccountLifecycle;
