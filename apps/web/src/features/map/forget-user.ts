import { forgetUserMap } from "@tokuly/core/map/forget";
import type { ForgetUser } from "#/seams/forget-user";
import { useLocations } from "#/stores/locations";

/**
 * After an unfriend or a block: their pin leaves the map, and so do
 * directions to them, bumps and their map stories; what friends share on the
 * map (gatherings, trips, SOS, alerts) is read again.
 */
export default ((queryClient, userId) => {
  useLocations.getState().remove(userId);
  void forgetUserMap(queryClient, userId);
}) satisfies ForgetUser;
