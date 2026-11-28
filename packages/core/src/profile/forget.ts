import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { profileKeys } from "./keys";

/** A friendship ended: profiles are read again (the relationship, friends-only fields). */
export function forgetUserProfile(queryClient: QueryClient, _userId: string): Promise<void> {
  return reread(queryClient, [profileKeys.profiles]);
}
