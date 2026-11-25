import { identity } from "@tokuly/api";
import type { ClientSource } from "../query";
import { profileKeys } from "./keys";

/** A person's profile by handle: who they are, how the viewer is connected, the domains' cards. */
export function profileQuery(client: ClientSource<typeof identity.ProfileService>, handle: string) {
  return {
    queryKey: profileKeys.profile(handle),
    queryFn: () => client().getProfile({ by: { case: "handle", value: handle } }),
  };
}

/** How the viewer stands to the profile's person. */
export function relationshipOf(profile: Pick<identity.Profile, "relationship"> | undefined) {
  return {
    isSelf: profile?.relationship === identity.Relationship.SELF,
    isFriend: profile?.relationship === identity.Relationship.FRIENDS,
  };
}
