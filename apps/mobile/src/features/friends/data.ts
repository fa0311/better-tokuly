import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { FRIEND_PUSHES, followKeys, friendKeys, friendQueries } from "@tokuly/core/friends";
import { reread, rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { Alert } from "react-native";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import { forgetUser } from "#/seams/forget-user";

export const friendClient = () => api().service(social.FriendService);
export const followClient = () => api().service(social.FollowService);

/**
 * Whether the account in use is locked (鍵アカウント; accounts of minors are
 * created locked): what it shares publicly reaches only its followers and friends.
 */
export function useAccountLocked(): boolean {
  const settings = useQuery({
    queryKey: followKeys.settings,
    queryFn: () => followClient().getFollowSettings({}),
    staleTime: 60_000,
  });
  return settings.data?.settings?.locked ?? false;
}

export const useFriends = (query: string) => useQuery(friendQueries.list(friendClient, query));
export const useIncomingRequests = () => useQuery(friendQueries.incoming(friendClient));
export const useOutgoingRequests = () => useQuery(friendQueries.outgoing(friendClient));
export const useMyInvite = () => useQuery(friendQueries.invite(friendClient));

const t = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key, { ns: "friends", ...options });

/** After any answer or request everything social is read again (`friendKeys.all`). */
function useSocialMutation<T>(
  run: (input: T) => Promise<unknown>,
  done: (input: T) => string | null,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: run,
    onSuccess: (_result, input) => {
      const message = done(input);
      if (message) Alert.alert(message);
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "friends" })),
    onSettled: () => queryClient.invalidateQueries({ queryKey: friendKeys.all }),
  });
}

export const useRespond = () =>
  useSocialMutation(
    ({ requestId, accept }: { requestId: string; accept: boolean }) =>
      friendClient().respondFriendRequest({ requestId, accept }),
    ({ accept }) => (accept ? t("toast.accepted") : null),
  );

export const useCancelRequest = () =>
  useSocialMutation(
    (requestId: string) => friendClient().cancelFriendRequest({ requestId }),
    () => t("toast.cancelled"),
  );

export const useSendRequest = () =>
  useSocialMutation(
    (target: { handle: string } | { userId: string; source: social.FriendSource }) =>
      friendClient().sendFriendRequest(
        "handle" in target
          ? {
              target: { case: "handle", value: target.handle.replace(/^@/, "").trim() },
              source: social.FriendSource.HANDLE,
            }
          : { target: { case: "userId", value: target.userId }, source: target.source },
      ),
    () => t("toast.sent"),
  );

/**
 * Requests arrive and are answered while the app is open: the lists follow
 * the pushes, and a new connection reads them again. A friendship that ended
 * (unfriended, blocked, or the account was deleted, by either side) takes
 * that person out of every feature at once (`#/seams/forget-user`).
 */
export function useFriendsSync(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    // What changed before the connection was up, or while it was down, was not announced.
    const offWelcome = rereadOnWelcome(queryClient, client, [friendKeys.all]);
    const off = client.onFrame((frame) => {
      if (frame.topic || frame.body.case !== "social") return;
      const event = frame.body.value.event;
      if (event.case === "friendRemovedUserId") forgetUser(queryClient, event.value);
      else if (FRIEND_PUSHES.has(event.case ?? "")) void reread(queryClient, [friendKeys.all]);
    });
    client.start();
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient]);
}
