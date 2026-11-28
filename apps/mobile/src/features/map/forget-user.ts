import { mapKeys } from "@tokuly/core/map";
import { reread } from "@tokuly/core/query";
import type { ForgetUser } from "#/seams/forget-user";
import { useLocations } from "./locations-store";

/** A friendship ended: their pin leaves the map, and the map's friends are read again. */
export default ((queryClient, userId) => {
  useLocations.getState().remove(userId);
  void reread(queryClient, [mapKeys.friends]);
}) satisfies ForgetUser;
