import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { moments } from "@tokuly/api";
import { pushTo, reread } from "../query";
import { momentKeys } from "./keys";

/**
 * The moment caches kept in step with what the caller does and what realtime
 * announces. Every change goes through the journal (`pushTo`), so a list read
 * in flight when it was pushed (read again on a Welcome, the map's refresh)
 * does not undo it: a deleted moment, or a person forgotten, does not come
 * back. What only has to be read again is `reread`.
 */

type FriendPages = InfiniteData<moments.ListFriendMomentsResponse>;

/** `edit` applied to the friends' list and the map; a list it leaves as it was is not written. */
function editMoments(
  queryClient: QueryClient,
  edit: (list: moments.Moment[]) => moments.Moment[],
): void {
  pushTo<FriendPages>(queryClient, momentKeys.friends, (data) => {
    const pages = data.pages.map((page) => {
      const list = edit(page.moments);
      return list === page.moments ? page : { ...page, moments: list };
    });
    return pages.some((page, index) => page !== data.pages[index]) ? { ...data, pages } : data;
  });
  pushTo<moments.ListMapMomentsResponse>(queryClient, momentKeys.map, (data) => {
    const list = edit(data.moments);
    return list === data.moments ? data : { ...data, moments: list };
  });
}

/** The list without the moments `drop` picks; the same list when that is none. */
const without =
  (drop: (moment: moments.Moment) => boolean) =>
  (list: moments.Moment[]): moments.Moment[] =>
    list.some(drop) ? list.filter((moment) => !drop(moment)) : list;

/** Applies `update` to a moment wherever it is cached: the friends' list, the map, the moment itself, the caller's own today. */
export function patchMoment(
  queryClient: QueryClient,
  momentId: string,
  update: (moment: moments.Moment) => moments.Moment,
): void {
  const apply = (moment: moments.Moment) => (moment.id === momentId ? update(moment) : moment);
  editMoments(queryClient, (list) => {
    const next = list.map(apply);
    return next.some((moment, index) => moment !== list[index]) ? next : list;
  });
  pushTo<moments.Moment>(queryClient, momentKeys.moment(momentId), apply);
  pushTo<moments.GetTodayResponse>(queryClient, momentKeys.today, (today) => {
    const mine = today.mine && apply(today.mine);
    return mine === today.mine ? today : { ...today, mine };
  });
}

/**
 * The caller posted their moment of the day: it is today's, and friends'
 * moments unlock (they are read again, now with their pictures).
 */
export function rememberPostedMoment(queryClient: QueryClient, moment: moments.Moment): void {
  pushTo<moments.GetTodayResponse>(queryClient, momentKeys.today, (today) => ({
    ...today,
    mine: moment,
    posted: true,
    unlocked: true,
  }));
  // A new moment: nothing can be reading it yet.
  queryClient.setQueryData(momentKeys.moment(moment.id), moment);
  void reread(queryClient, [momentKeys.friends, momentKeys.map, momentKeys.calendars]);
}

/**
 * A moment is gone (deleted by its owner, taken down). When it was the
 * caller's own of the day, friends' moments lock again: the day stays used.
 */
export function removeMoment(queryClient: QueryClient, momentId: string): void {
  editMoments(
    queryClient,
    without((moment) => moment.id === momentId),
  );
  queryClient.removeQueries({ queryKey: momentKeys.moment(momentId), exact: true });
  queryClient.removeQueries({ queryKey: momentKeys.reactions(momentId), exact: true });
  const mine =
    queryClient.getQueryData<moments.GetTodayResponse>(momentKeys.today)?.mine?.id === momentId;
  pushTo<moments.GetTodayResponse>(queryClient, momentKeys.today, (today) =>
    today.mine?.id === momentId ? { ...today, mine: undefined, unlocked: false } : today,
  );
  void reread(queryClient, [
    ...(mine ? [momentKeys.today, momentKeys.friends, momentKeys.map] : []),
    momentKeys.calendars,
  ]);
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
  void reread(queryClient, [momentKeys.reactions(momentId)]);
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
 * list and the map — also those a read in flight brings — and nothing of
 * them stays cached, reactions included.
 */
export function forgetUserMoments(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  const theirs = (moment: moments.Moment) => moment.author?.id === userId;
  const cached = cachedMoments(queryClient).filter(theirs);
  editMoments(queryClient, without(theirs));
  for (const { id } of cached) {
    queryClient.removeQueries({ queryKey: momentKeys.moment(id), exact: true });
    queryClient.removeQueries({ queryKey: momentKeys.reactions(id), exact: true });
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
      pushTo<moments.GetTodayResponse>(queryClient, momentKeys.today, (today) => ({
        ...today,
        day: event.value,
        mine: undefined,
        posted: false,
        unlocked: false,
      }));
      void reread(queryClient, [momentKeys.today, momentKeys.friends, momentKeys.map]);
      break;
    case "friendPosted":
      // Read as the caller may see it: locked or not, with or without its place.
      void reread(queryClient, [momentKeys.friends, momentKeys.map]);
      break;
    case "deleted":
      removeMoment(queryClient, event.value.momentId);
      break;
    case "reactionsChanged": {
      const { momentId, reactionCount } = event.value;
      patchMoment(queryClient, momentId, (moment) =>
        moment.reactionCount === reactionCount ? moment : { ...moment, reactionCount },
      );
      void reread(queryClient, [momentKeys.moment(momentId), momentKeys.reactions(momentId)]);
      break;
    }
    default:
      break;
  }
}
