import { forgetUserMap } from "@tokuly/core/map/forget";
import type { ForgetUser } from "#/seams/forget-user";
import { useLocations } from "./locations-store";

/** A friendship ended: their pin leaves the map, and what friends share there is read again. */
export default ((queryClient, userId) => {
  useLocations.getState().remove(userId);
  void forgetUserMap(queryClient, userId);
}) satisfies ForgetUser;
