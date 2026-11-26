import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { community } from "@tokuly/api";
import {
  communityQueries,
  type DirectoryOptions,
  type ThreadListOptions,
  uniqueById,
} from "@tokuly/core/community";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * Communities in the query cache. The keys, query options and cache edits
 * are `@tokuly/core/community`; this file wires them to the phone's API
 * client. Every client is `api()` when called: whoever is signed in now.
 */
export const communityClient = () => api().service(community.CommunityService);
export const boardClient = () => api().service(community.CommunityBoardService);
export const discoveryClient = () => api().service(community.CommunityDiscoveryService);

/**
 * One write as the account signed in now (`send` is called at once, before
 * any await). Its answer — or its refusal — belongs to that account: once
 * another one took over it is `undefined`, so nothing of it is shown or
 * opened under the next account (README, "Accounts").
 */
export async function asAccount<T>(send: () => Promise<T>): Promise<T | undefined> {
  const account = currentAccount();
  if (!account) return undefined;
  try {
    const answer = await send();
    return isActive(account) ? answer : undefined;
  } catch (error) {
    if (isActive(account)) throw error;
    return undefined;
  }
}

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "community";

/** One community as the caller sees it; `null` when it does not exist for them. */
export function useCommunity(idOrHandle: string) {
  return useQuery({
    ...communityQueries.community(communityClient, idOrHandle),
    enabled: idOrHandle !== "",
  });
}

/** The caller's communities and the requests they wait on. */
export function useMyCommunities() {
  const query = useInfiniteQuery(communityQueries.my(communityClient));
  const pages = query.data?.pages;
  return {
    ...query,
    communities: pages?.flatMap((page) => page.communities),
    requested: pages?.[0]?.requested ?? [],
  };
}

export function useDirectory(options: DirectoryOptions) {
  return useInfiniteQuery(communityQueries.directory(discoveryClient, options));
}

export function useTrending(category: community.CommunityCategory) {
  return useQuery(communityQueries.trending(discoveryClient, category));
}

export function useFriendsCommunities() {
  return useInfiniteQuery(communityQueries.friends(discoveryClient));
}

export function useMembers(communityId: string, query: string, role: community.MemberRole) {
  return useInfiniteQuery({
    ...communityQueries.members(communityClient, communityId, query, role),
    enabled: communityId !== "",
  });
}

export function useInvitePreview(code: string) {
  return useQuery(communityQueries.invitePreview(communityClient, code));
}

export function useChannels(communityId: string, enabled = true) {
  return useQuery({ ...communityQueries.channels(boardClient, communityId), enabled });
}

/** A board: the pinned threads on top, then the list (the hot order may repeat one across pages). */
export function useThreads(communityId: string, options: ThreadListOptions, enabled = true) {
  const query = useInfiniteQuery({
    ...communityQueries.threads(boardClient, communityId, options),
    enabled,
  });
  const pages = query.data?.pages;
  return {
    ...query,
    pinned: pages?.[0]?.pinned ?? [],
    threads: pages && uniqueById(pages.flatMap((page) => page.items)),
  };
}

export function useAnnouncements(communityId: string, enabled = true) {
  return useInfiniteQuery({ ...communityQueries.announcements(boardClient, communityId), enabled });
}

/** One thread by UUID or short ID; `null` when it is gone or not the caller's to read. */
export function useThread(threadId: string) {
  return useQuery(communityQueries.thread(boardClient, threadId));
}

/** A thread's replies, oldest first. */
export function useReplies(threadUuid: string, enabled = true) {
  return useInfiniteQuery({ ...communityQueries.replies(boardClient, threadUuid, false), enabled });
}

/** Every item of a paged list. */
export function itemsOf<T>(data: { pages: { items: T[] }[] } | undefined): T[] {
  return data?.pages.flatMap((page) => page.items) ?? [];
}
