import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { api } from "#/lib/api";

export const boxClient = () => api().service(ask.QuestionBoxService);
export const promptClient = () => api().service(ask.PromptService);
export const complimentClient = () => api().service(ask.ComplimentService);

export const PAGE_SIZE = 20;

export const askKeys = {
  all: ["ask"] as const,
  settings: ["ask", "settings"] as const,
  box: (user: string) => ["ask", "box", user] as const,
  counts: ["ask", "counts"] as const,
  inboxAll: ["ask", "inbox"] as const,
  inbox: (folder: ask.QuestionFolder) => ["ask", "inbox", folder] as const,
  asked: ["ask", "asked"] as const,
  answersAll: ["ask", "answers"] as const,
  answers: (user: string) => ["ask", "answers", user] as const,
  question: (id: string) => ["ask", "question", id] as const,
  blocks: ["ask", "blocks"] as const,
  promptsAll: ["ask", "prompts"] as const,
  promptsOf: (user: string) => ["ask", "prompts", "of", user] as const,
  friendPrompts: ["ask", "prompts", "friends"] as const,
  prompt: (id: string) => ["ask", "prompt", id] as const,
  responsesAll: (promptId: string) => ["ask", "responses", promptId] as const,
  responses: (promptId: string, filter: ask.ResponseFilter) =>
    ["ask", "responses", promptId, filter] as const,
  round: ["ask", "round"] as const,
  flames: ["ask", "flames"] as const,
  flameSummary: ["ask", "flameSummary"] as const,
  topFlames: ["ask", "topFlames"] as const,
};

type Paged<T> = InfiniteData<T, string>;
const cursorOf = (page: { page?: { nextCursor: string; hasMore: boolean } }) =>
  page.page?.hasMore ? page.page.nextCursor : undefined;
const pageRequest = (cursor: string) => ({ cursor, limit: PAGE_SIZE });

// ───────────────────────────── Question box ─────────────────────────────

export function useSettings() {
  return useQuery({
    queryKey: askKeys.settings,
    queryFn: async () => (await boxClient().getSettings({})).settings!,
  });
}

/** A person's box as the viewer sees it; `null` when there is none to show (no such person, or a block). */
export function useBox(user: string | undefined) {
  return useQuery({
    queryKey: askKeys.box(user ?? ""),
    queryFn: async () => (await boxClient().getBox({ user: user! })).box ?? null,
    enabled: Boolean(user),
  });
}

export function useInboxCounts(enabled = true) {
  return useQuery({
    queryKey: askKeys.counts,
    queryFn: async () => (await boxClient().getInboxCounts({})).counts!,
    staleTime: 5 * 60_000,
    enabled,
  });
}

export function useInbox(folder: ask.QuestionFolder) {
  return useInfiniteQuery({
    queryKey: askKeys.inbox(folder),
    queryFn: ({ pageParam }) => boxClient().listInbox({ folder, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  });
}

export function useAskedQuestions() {
  return useInfiniteQuery({
    queryKey: askKeys.asked,
    queryFn: ({ pageParam }) => boxClient().listAskedQuestions({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  });
}

/** Someone's Q&A page. */
export function useAnswers(user: string | undefined) {
  return useInfiniteQuery({
    queryKey: askKeys.answers(user ?? ""),
    queryFn: ({ pageParam }) =>
      boxClient().listAnswers({ user: user!, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
    enabled: Boolean(user),
  });
}

export function useQuestion(id: string) {
  return useQuery({
    queryKey: askKeys.question(id),
    queryFn: async () => (await boxClient().getQuestion({ questionId: id })).question!,
  });
}

export function useSenderBlocks() {
  return useInfiniteQuery({
    queryKey: askKeys.blocks,
    queryFn: ({ pageParam }) => boxClient().listSenderBlocks({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  });
}

/** Puts the counts a response carried into the cache (the badge and the tabs read them). */
export function rememberCounts(
  queryClient: QueryClient,
  counts: ask.InboxCounts | undefined,
): void {
  if (counts) queryClient.setQueryData(askKeys.counts, counts);
}

/** Takes a question out of every loaded inbox folder at once, so the row leaves under the finger. */
export function dropFromInbox(queryClient: QueryClient, questionId: string): void {
  queryClient.setQueriesData<Paged<ask.ListInboxResponse>>(
    { queryKey: askKeys.inboxAll },
    (data) =>
      data
        ? {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              questions: page.questions.filter((question) => question.id !== questionId),
            })),
          }
        : data,
  );
}

/** A question that just arrived goes on top of the loaded inbox. */
export function addToInbox(queryClient: QueryClient, question: ask.Question): void {
  queryClient.setQueryData<Paged<ask.ListInboxResponse>>(
    askKeys.inbox(ask.QuestionFolder.INBOX),
    (data) => {
      if (
        !data?.pages[0] ||
        data.pages.some((page) => page.questions.some((known) => known.id === question.id))
      )
        return data;
      const [first, ...rest] = data.pages;
      return { ...data, pages: [{ ...first, questions: [question, ...first.questions] }, ...rest] };
    },
  );
}

/** After anything that changed where questions sit: lists and counts are read again. */
export function refreshInbox(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: askKeys.inboxAll });
  void queryClient.invalidateQueries({ queryKey: askKeys.counts });
}

/** After an answer appeared, changed or went away. */
export function refreshAnswers(queryClient: QueryClient, question?: ask.Question): void {
  refreshInbox(queryClient);
  void queryClient.invalidateQueries({ queryKey: askKeys.answersAll });
  if (question) queryClient.setQueryData(askKeys.question(question.id), question);
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
