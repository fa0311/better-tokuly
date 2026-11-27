import { social } from "@tokuly/api";
import type { ClientSource } from "../query";
import { friendKeys } from "./keys";

type Friends = ClientSource<typeof social.FriendService>;

/** Query options of friends. Each app adds its hooks around them. */
export const friendQueries = {
  /** The viewer's friends, optionally filtered by name or handle. */
  list: (client: Friends, query = "") => ({
    queryKey: friendKeys.list(query),
    queryFn: () => client().listFriends({ query, page: { limit: 100 } }),
  }),

  /** Friend requests waiting for the viewer's answer. */
  incoming: (client: Friends) => ({
    queryKey: friendKeys.incoming,
    queryFn: () =>
      client().listFriendRequests({ direction: social.FriendRequestDirection.INCOMING }),
  }),

  /** Friend requests the viewer sent that wait for an answer. */
  outgoing: (client: Friends) => ({
    queryKey: friendKeys.outgoing,
    queryFn: () =>
      client().listFriendRequests({ direction: social.FriendRequestDirection.OUTGOING }),
  }),

  /** People the viewer may know. */
  suggestions: (client: Friends) => ({
    queryKey: friendKeys.suggestionList(20),
    queryFn: () => client().listSuggestions({ limit: 20 }),
  }),

  /** The viewer's invite link. */
  invite: (client: Friends) => ({
    queryKey: friendKeys.invite,
    queryFn: () => client().getMyInvite({}),
  }),
};
