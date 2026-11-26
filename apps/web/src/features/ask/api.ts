import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { askKeys, boxQueries, PAGE_SIZE } from "@tokuly/core/ask";
import { api } from "#/lib/api";

export const boxClient = () => api().service(ask.QuestionBoxService);
export const promptClient = () => api().service(ask.PromptService);
export const complimentClient = () => api().service(ask.ComplimentService);

type Paged<T> = InfiniteData<T, string>;
const cursorOf = (page: { page?: { nextCursor: string; hasMore: boolean } }) =>
  page.page?.hasMore ? page.page.nextCursor : undefined;
const pageRequest = (cursor: string) => ({ cursor, limit: PAGE_SIZE });

// ───────────────────────────── Question box ─────────────────────────────

export function useSettings() {
  return useQuery(boxQueries.settings(boxClient));
}

/** A person's box as the viewer sees it; `null` when there is none to show (no such person, or a block). */
export function useBox(user: string | undefined) {
  return useQuery({ ...boxQueries.box(boxClient, user ?? ""), enabled: Boolean(user) });
}

export function useInboxCounts(enabled = true) {
  return useQuery({ ...boxQueries.counts(boxClient), enabled });
}

export function useInbox(folder: ask.QuestionFolder) {
  return useInfiniteQuery(boxQueries.inbox(boxClient, folder));
}

export function useAskedQuestions() {
  return useInfiniteQuery(boxQueries.asked(boxClient));
}

/** Someone's Q&A page. */
export function useAnswers(user: string | undefined) {
  return useInfiniteQuery({
    ...boxQueries.answers(boxClient, user ?? ""),
    enabled: Boolean(user),
  });
}

export function useQuestion(id: string) {
  return useQuery(boxQueries.question(boxClient, id));
}

export function useSenderBlocks() {
  return useInfiniteQuery(boxQueries.blocks(boxClient));
}

// ───────────────────────────── Prompts ─────────────────────────────

/** A person's prompts: the viewer's own when `user` is empty. */
export function usePromptsOf(user: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: askKeys.promptsOf(user),
    queryFn: ({ pageParam }) => promptClient().listPrompts({ user, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
    enabled,
  });
}

export function useFriendPrompts() {
  return useInfiniteQuery({
    queryKey: askKeys.friendPrompts,
    queryFn: ({ pageParam }) => promptClient().listFriendPrompts({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  });
}

export function usePrompt(id: string) {
  return useQuery({
    queryKey: askKeys.prompt(id),
    queryFn: () => promptClient().getPrompt({ promptId: id }),
  });
}

export function usePromptResponses(promptId: string, filter: ask.ResponseFilter, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: askKeys.responses(promptId, filter),
    queryFn: ({ pageParam }) =>
      promptClient().listPromptResponses({ promptId, filter, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
    enabled,
  });
}

export function refreshPrompt(queryClient: QueryClient, promptId: string): void {
  void queryClient.invalidateQueries({ queryKey: askKeys.prompt(promptId) });
  void queryClient.invalidateQueries({ queryKey: askKeys.responsesAll(promptId) });
  void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
}

// ───────────────────────────── Compliment polls ─────────────────────────────

export function useFlames() {
  return useInfiniteQuery({
    queryKey: askKeys.flames,
    queryFn: ({ pageParam }) => complimentClient().listFlames({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  });
}

export function useFlameSummary(enabled = true) {
  return useQuery({
    queryKey: askKeys.flameSummary,
    queryFn: async () => (await complimentClient().getFlameSummary({})).summary!,
    staleTime: 5 * 60_000,
    enabled,
  });
}

export function useTopFlames() {
  return useQuery({
    queryKey: askKeys.topFlames,
    queryFn: () => complimentClient().getTopFlames({}),
  });
}

/** Replaces one flame in the loaded list (a hint was bought). */
export function rememberFlame(queryClient: QueryClient, flame: ask.Flame): void {
  queryClient.setQueryData<Paged<ask.ListFlamesResponse>>(askKeys.flames, (data) =>
    data
      ? {
          ...data,
          pages: data.pages.map((page) => ({
            ...page,
            flames: page.flames.map((known) => (known.id === flame.id ? flame : known)),
          })),
        }
      : data,
  );
}
