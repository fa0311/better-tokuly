import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { ask } from "@tokuly/api";
import { askKeys } from "./keys";

/**
 * The question box in the query cache: counts a response carried, questions
 * leaving or joining the loaded inbox, what to read again after a change, and
 * what a pushed `AskEvent` changes.
 */

type Paged<T> = InfiniteData<T, string>;

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

/**
 * Applies a realtime `AskEvent` to what the app holds. Nothing is announced
 * here: each of these also arrives as a notification.
 */
export function applyAskEvent(queryClient: QueryClient, event: ask.AskEvent["event"]): void {
  switch (event.case) {
    case "questionReceived": {
      rememberCounts(queryClient, event.value.counts);
      if (event.value.question) addToInbox(queryClient, event.value.question);
      break;
    }
    case "questionAnswered": {
      const question = event.value.question;
      if (question) queryClient.setQueryData(askKeys.question(question.id), question);
      void queryClient.invalidateQueries({ queryKey: askKeys.asked });
      break;
    }
    case "flameReceived": {
      const unseen = event.value.unseen;
      queryClient.setQueryData<ask.FlameSummary>(askKeys.flameSummary, (known) =>
        known ? { ...known, unseen } : known,
      );
      void queryClient.invalidateQueries({ queryKey: askKeys.flameSummary });
      void queryClient.invalidateQueries({ queryKey: askKeys.flames });
      void queryClient.invalidateQueries({ queryKey: askKeys.topFlames });
      break;
    }
    case "promptResponseReceived": {
      const promptId = event.value.prompt?.id ?? event.value.response?.promptId;
      if (!promptId) break;
      void queryClient.invalidateQueries({ queryKey: askKeys.prompt(promptId) });
      void queryClient.invalidateQueries({ queryKey: askKeys.responsesAll(promptId) });
      void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
      break;
    }
    default:
      break;
  }
}
