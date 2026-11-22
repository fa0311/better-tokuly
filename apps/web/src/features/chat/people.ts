import { useQuery } from "@tanstack/react-query";
import { type chat, type common, social } from "@tokuly/api";
import { api, profileClient } from "#/lib/api";
import i18n from "#/lib/i18n";
import { useLive } from "./live";
import { isGroup, peerOf } from "./model";

const friendClient = () => api().service(social.FriendService);

/**
 * The viewer's friends, optionally filtered by name or handle. Shares its
 * cache entry with the friends screen.
 */
export function useFriends(query = "") {
  return useQuery({
    queryKey: ["social", "friends", "list", query],
    queryFn: () => friendClient().listFriends({ query, page: { limit: 100 } }),
  });
}

/** Whether a person has the app open: the latest presence event, else what the friend list said. */
export function useOnline(userId: string | undefined): boolean {
  const live = useLive((state) => (userId ? state.online[userId] : undefined));
  const friends = useFriends();
  if (!userId) return false;
  return (
    live ?? friends.data?.friends.find((friend) => friend.user?.id === userId)?.online ?? false
  );
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The user ID behind a handle (`aoi`, `@aoi`) or a user ID. Rejects with USER_NOT_FOUND. */
export async function resolveUserId(handleOrUserId: string): Promise<string> {
  const value = handleOrUserId.trim();
  if (UUID.test(value)) return value;
  const { profile } = await profileClient().getProfile({
    by: { case: "handle", value: value.replace(/^@/, "") },
  });
  return profile?.user?.id ?? "";
}

/** A person's name for headings and rows. */
export function displayName(user: common.UserSummary | undefined): string {
  if (!user || user.deleted) return i18n.t("people.deleted", { ns: "chat" });
  return user.displayName || `@${user.handle}`;
}

/** What a conversation is called: the group's name, or the other person. */
export function titleOf(conversation: chat.Conversation, myId: string): string {
  if (isGroup(conversation)) return conversation.name || i18n.t("group.unnamed", { ns: "chat" });
  return displayName(peerOf(conversation, myId));
}

/** Whether a conversation matches a search typed into the list (name, member names, handles). */
export function matchesQuery(
  conversation: chat.Conversation,
  myId: string,
  query: string,
): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  const haystack = [
    conversation.name,
    ...conversation.members
      .filter((member) => member.id !== myId)
      .flatMap((member) => [member.displayName, member.handle]),
  ];
  return haystack.some((value) => value.toLowerCase().includes(needle.replace(/^@/, "")));
}
