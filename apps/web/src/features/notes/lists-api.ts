import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { identity, notes } from "@tokuly/api";
import { listKeys, notesKeys } from "@tokuly/core/notes";
import { api } from "#/lib/api";

export const listClient = () => api().service(notes.NoteListService);
export const antennaClient = () => api().service(notes.AntennaService);

/** Limits the server enforces, so forms can say so before asking. */
export const LIST_LIMITS = { name: 50, description: 200, lists: 50, members: 500 } as const;
export const ANTENNA_LIMITS = {
  name: 30,
  antennas: 5,
  groups: 10,
  words: 5,
  exclude: 10,
  users: 50,
} as const;

/** The lists of a person (all of them for oneself, public ones for others), in the owner's order. */
export function useLists(userId?: string) {
  return useQuery({
    queryKey: userId ? listKeys.of(userId) : listKeys.mine,
    queryFn: async () => (await listClient().listLists({ userId: userId ?? "" })).lists,
  });
}

export function useList(listId: string) {
  return useQuery({
    queryKey: listKeys.list(listId),
    queryFn: async () => (await listClient().getList({ listId })).list ?? null,
  });
}

export function useListMembers(listId: string) {
  return useInfiniteQuery({
    queryKey: listKeys.members(listId),
    queryFn: ({ pageParam }) =>
      listClient().listListMembers({ listId, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

export function useFollowedLists(enabled = true) {
  return useInfiniteQuery({
    queryKey: listKeys.followed,
    queryFn: ({ pageParam }) =>
      listClient().listFollowedLists({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

export function useListsContainingMe(enabled = true) {
  return useInfiniteQuery({
    queryKey: listKeys.containingMe,
    queryFn: ({ pageParam }) =>
      listClient().listListsContainingMe({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

/** People to add to a list, by name or handle. */
export function useUserSearch(query: string) {
  return useQuery({
    queryKey: listKeys.userSearch(query),
    queryFn: async () =>
      (
        await api()
          .service(identity.ProfileService)
          .searchUsers({ query, page: { limit: 20 } })
      ).results,
    enabled: query.trim().length > 0,
    staleTime: 30_000,
  });
}

/** A list changed (or appeared): every cached copy of it takes the new state. */
export function storeList(queryClient: QueryClient, list: notes.NoteList | undefined): void {
  if (!list) return;
  queryClient.setQueryData(listKeys.list(list.id), list);
  const replace = (lists: notes.NoteList[] | undefined) =>
    lists?.map((item) => (item.id === list.id ? list : item));
  for (const query of queryClient.getQueryCache().findAll({ queryKey: ["notes", "lists", "of"] })) {
    queryClient.setQueryData<notes.NoteList[]>(query.queryKey, replace);
  }
  for (const key of [listKeys.followed, listKeys.containingMe]) {
    queryClient.setQueryData<
      InfiniteData<notes.ListFollowedListsResponse | notes.ListListsContainingMeResponse>
    >(
      key,
      (data) =>
        data && {
          ...data,
          pages: data.pages.map((page) => ({ ...page, lists: replace(page.lists) ?? [] })),
        },
    );
  }
}

/** A list's members or audience changed: its timeline is read again. */
export function resetListTimeline(queryClient: QueryClient, listId: string): Promise<void> {
  return queryClient.resetQueries({ queryKey: notesKeys.timeline({ kind: "list", listId }) });
}

export function useAntennas() {
  return useQuery({
    queryKey: listKeys.antennas,
    queryFn: async () => (await antennaClient().listAntennas({})).antennas,
  });
}

export function useAntenna(antennaId: string) {
  return useQuery({
    queryKey: listKeys.antenna(antennaId),
    queryFn: async () => (await antennaClient().getAntenna({ antennaId })).antenna ?? null,
  });
}
