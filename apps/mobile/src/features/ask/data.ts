import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { askKeys, boxQueries } from "@tokuly/core/ask";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * The question box in the query cache. The keys, query options and cache
 * edits are `@tokuly/core/ask`; this file wires them to the phone's API
 * client. Every client is `api()` when called: whoever is signed in now.
 * Nothing here names an anonymous sender: keys hold the viewer's own things
 * and item IDs, and what is cached is what the server returned.
 */
export const boxClient = () => api().service(ask.QuestionBoxService);
const complimentClient = () => api().service(ask.ComplimentService);

/**
 * The account a change to the box is made as, taken before anything awaits:
 * its client, and whether it is still the one in use once the answer is back
 * (only then is the answer shown).
 */
export function boxAs(): { account: Account | null; client: ReturnType<typeof boxClient> } {
  const account = currentAccount();
  return { account, client: account ? account.api.service(ask.QuestionBoxService) : boxClient() };
}

export const stillActive = (account: Account | null): boolean =>
  account !== null && isActive(account);

export function useSettings() {
  return useQuery(boxQueries.settings(boxClient));
}

/** A person's box (ID or handle) as the viewer sees it; `null` when there is none to show. */
export function useBox(user: string) {
  return useQuery({ ...boxQueries.box(boxClient, user), enabled: user !== "" });
}

export function useInboxCounts() {
  return useQuery(boxQueries.counts(boxClient));
}

export function useInbox(folder: ask.QuestionFolder) {
  return useInfiniteQuery(boxQueries.inbox(boxClient, folder));
}

export function useAskedQuestions() {
  return useInfiniteQuery(boxQueries.asked(boxClient));
}

/** Someone's Q&A page, by user ID. */
export function useAnswers(userId: string) {
  return useInfiniteQuery({ ...boxQueries.answers(boxClient, userId), enabled: userId !== "" });
}

export function useQuestion(id: string) {
  return useQuery({ ...boxQueries.question(boxClient, id), enabled: id !== "" });
}

/** Flames the viewer has not looked at yet (the polls themselves are on the web). */
export function useUnseenFlames(): number {
  return (
    useQuery({
      queryKey: askKeys.flameSummary,
      queryFn: async () => (await complimentClient().getFlameSummary({})).summary!,
      staleTime: 5 * 60_000,
    }).data?.unseen ?? 0
  );
}
