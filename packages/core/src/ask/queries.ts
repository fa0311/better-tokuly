import type { ask } from "@tokuly/api";
import type { ClientSource } from "../query";
import { askKeys } from "./keys";

type Box = ClientSource<typeof ask.QuestionBoxService>;

/** Items per page of every list of this domain. */
export const PAGE_SIZE = 20;

const cursorOf = (page: { page?: { nextCursor: string; hasMore: boolean } }) =>
  page.page?.hasMore ? page.page.nextCursor : undefined;
const pageRequest = (cursor: string) => ({ cursor, limit: PAGE_SIZE });

/** Query options of the question box. Each app adds `enabled` and its hooks around them. */
export const boxQueries = {
  /** The viewer's own box settings. */
  settings: (client: Box) => ({
    queryKey: askKeys.settings,
    queryFn: async () => (await client().getSettings({})).settings!,
  }),

  /** A person's box (by ID or handle) as the viewer sees it; `null` when there is none to show. */
  box: (client: Box, user: string) => ({
    queryKey: askKeys.box(user),
    queryFn: async (): Promise<ask.QuestionBox | null> =>
      (await client().getBox({ user })).box ?? null,
  }),

  counts: (client: Box) => ({
    queryKey: askKeys.counts,
    queryFn: async () => (await client().getInboxCounts({})).counts!,
    staleTime: 5 * 60_000,
  }),

  inbox: (client: Box, folder: ask.QuestionFolder) => ({
    queryKey: askKeys.inbox(folder),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listInbox({ folder, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  }),

  /** The questions the viewer sent: "waiting" or "answered", nothing else. */
  asked: (client: Box) => ({
    queryKey: askKeys.asked,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listAskedQuestions({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  }),

  /** Someone's Q&A page (by user ID). */
  answers: (client: Box, user: string) => ({
    queryKey: askKeys.answers(user),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listAnswers({ user, page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  }),

  question: (client: Box, id: string) => ({
    queryKey: askKeys.question(id),
    queryFn: async () => (await client().getQuestion({ questionId: id })).question!,
  }),

  blocks: (client: Box) => ({
    queryKey: askKeys.blocks,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listSenderBlocks({ page: pageRequest(pageParam) }),
    initialPageParam: "",
    getNextPageParam: cursorOf,
  }),
};
