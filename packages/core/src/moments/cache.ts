import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { moments } from "@tokuly/api";
import { momentKeys } from "./keys";

/** The moment caches kept in step with what the caller does and what realtime announces. */

type FriendPages = InfiniteData<moments.ListFriendMomentsResponse>;

function editMoments(
  queryClient: QueryClient,
  edit: (list: moments.Moment[]) => moments.Moment[],
): void {
  queryClient.setQueryData<FriendPages>(
    momentKeys.friends,
    (data) =>
      data && {
        ...data,
        pages: data.pages.map((page) => ({ ...page, moments: edit(page.moments) })),
      },
  );
  queryClient.setQueryData<moments.ListMapMomentsResponse>(
    momentKeys.map,
    (data) => data && { ...data, moments: edit(data.moments) },
  );
}

/** Applies `update` to a moment wherever it is cached: the friends' list, the map, the moment itself, the caller's own today. */
export function patchMoment(
  queryClient: QueryClient,
  momentId: string,
  update: (moment: moments.Moment) => moments.Moment,
): void {
  const apply = (moment: moments.Moment) => (moment.id === momentId ? update(moment) : moment);
  editMoments(queryClient, (list) => list.map(apply));
  queryClient.setQueryData<moments.Moment>(
    momentKeys.moment(momentId),
    (moment) => moment && apply(moment),
  );
  queryClient.setQueryData<moments.GetTodayResponse>(
    momentKeys.today,
    (today) => today?.mine && { ...today, mine: apply(today.mine) },
  );
}

/**
 * The caller posted their moment of the day: it is today's, and friends'
 * moments unlock (they are read again, now with their pictures).
 */
export function rememberPostedMoment(queryClient: QueryClient, moment: moments.Moment): void {
  queryClient.setQueryData<moments.GetTodayResponse>(
    momentKeys.today,
    (today) => today && { ...today, mine: moment, posted: true, unlocked: true },
  );
  queryClient.setQueryData(momentKeys.moment(moment.id), moment);
  void queryClient.invalidateQueries({ queryKey: momentKeys.friends });
  void queryClient.invalidateQueries({ queryKey: momentKeys.map });
  void queryClient.invalidateQueries({ queryKey: momentKeys.calendars });
}

/**
 * A moment is gone (deleted by its owner, taken down). When it was the
 * caller's own of the day, friends' moments lock again: the day stays used.
 */
export function removeMoment(queryClient: QueryClient, momentId: string): void {
  editMoments(queryClient, (list) => list.filter((moment) => moment.id !== momentId));
  queryClient.removeQueries({ queryKey: momentKeys.moment(momentId), exact: true });
  queryClient.removeQueries({ queryKey: momentKeys.reactions(momentId), exact: true });
  const today = queryClient.getQueryData<moments.GetTodayResponse>(momentKeys.today);
  if (today?.mine?.id === momentId) {
    queryClient.setQueryData(momentKeys.today, { ...today, mine: undefined, unlocked: false });
    void queryClient.invalidateQueries({ queryKey: momentKeys.today });
    void queryClient.invalidateQueries({ queryKey: momentKeys.friends });
    void queryClient.invalidateQueries({ queryKey: momentKeys.map });
  }
  void queryClient.invalidateQueries({ queryKey: momentKeys.calendars });
}

/**
 * The caller's reaction on a moment changed (`reaction` unset: taken back).
 * The count and the first reactions follow; the full list is read again.
 */
export function setMyReaction(
  queryClient: QueryClient,
  momentId: string,
  reaction: moments.MomentReaction | undefined,
): void {
  patchMoment(queryClient, momentId, (moment) => {
    const before = moment.myReaction;
    const others = moment.reactions.filter((entry) => entry.id !== before?.id);
    const count = moment.reactionCount + (reaction ? 1 : 0) - (before ? 1 : 0);
    return {
      ...moment,
      myReaction: reaction,
      reactions: reaction ? [reaction, ...others] : others,
      reactionCount: Math.max(0, count),
    };
  });
  void queryClient.invalidateQueries({ queryKey: momentKeys.reactions(momentId) });
}

/** Every moment in the cache: the friends' list, the map and the moments read one by one. */
function cachedMoments(queryClient: QueryClient): moments.Moment[] {
  const friends = queryClient.getQueryData<FriendPages>(momentKeys.friends);
  const map = queryClient.getQueryData<moments.ListMapMomentsResponse>(momentKeys.map);
  const details = queryClient.getQueriesData<moments.Moment>({ queryKey: momentKeys.moments });
  return [
    ...(friends?.pages.flatMap((page) => page.moments) ?? []),
    ...(map?.moments ?? []),
    ...details.flatMap(([, moment]) => moment ?? []),
  ];
}

/**
 * After an unfriend or a block: that person's moments leave the friends'
 * list and the map, and nothing of them stays cached, reactions included.
 */
export function forgetUserMoments(queryClient: QueryClient, userId: string): void {
  const theirs = new Set(
    cachedMoments(queryClient)
      .filter((moment) => moment.author?.id === userId)
      .map((moment) => moment.id),
  );
  editMoments(queryClient, (list) => list.filter((moment) => !theirs.has(moment.id)));
  for (const momentId of theirs) {
    queryClient.removeQueries({ queryKey: momentKeys.moment(momentId), exact: true });
    queryClient.removeQueries({ queryKey: momentKeys.reactions(momentId), exact: true });
  }
}

/** What a pushed `MomentsEvent` changes in the cache. */
export function applyMomentsEvent(
  queryClient: QueryClient,
  event: moments.MomentsEvent["event"],
): void {
  switch (event.case) {
    case "prompt":
      // A new day: nothing posted yet, friends locked until the caller shares theirs.
      queryClient.setQueryData<moments.GetTodayResponse>(
        momentKeys.today,
        (today) =>
          today && { ...today, day: event.value, mine: undefined, posted: false, unlocked: false },
      );
      void queryClient.invalidateQueries({ queryKey: momentKeys.today });
      void queryClient.invalidateQueries({ queryKey: momentKeys.friends });
      void queryClient.invalidateQueries({ queryKey: momentKeys.map });
      break;
    case "friendPosted":
      // Read as the caller may see it: locked or not, with or without its place.
      void queryClient.invalidateQueries({ queryKey: momentKeys.friends });
      void queryClient.invalidateQueries({ queryKey: momentKeys.map });
      break;
    case "deleted":
      removeMoment(queryClient, event.value.momentId);
      break;
    case "reactionsChanged": {
      const { momentId, reactionCount } = event.value;
      patchMoment(queryClient, momentId, (moment) => ({ ...moment, reactionCount }));
      void queryClient.invalidateQueries({ queryKey: momentKeys.moment(momentId) });
      void queryClient.invalidateQueries({ queryKey: momentKeys.reactions(momentId) });
      break;
    }
    default:
      break;
  }
}
