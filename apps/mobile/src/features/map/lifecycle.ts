import type { AccountLifecycle } from "#/seams/lifecycle";
import { useLocations } from "./locations-store";
import { leaveSharing } from "./sharing";

/**
 * Nothing more is sent for the account, and the server hears that it stopped
 * sharing from here. Its friends' positions are not the next account's to see.
 */
export default {
  beforeLeave: leaveSharing,
  forget: () => useLocations.setState({ friends: {}, online: {}, own: null }),
} satisfies AccountLifecycle;
