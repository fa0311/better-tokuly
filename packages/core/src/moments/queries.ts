import type { moments } from "@tokuly/api";
import { type ClientSource, journal, nextCursor } from "../query";
import { momentKeys } from "./keys";

type Moments = ClientSource<typeof moments.MomentService>;
type Reactions = ClientSource<typeof moments.MomentReactionService>;

/**
 * Query options of moments. Each app adds `enabled` and its hooks around them.
 * What realtime changes reads through the journal (`pushTo` in `./cache`).
 */
export const momentQueries = {
  /** The caller's day. Its realtime `prompt` keeps it current; the clock does the rest. */
  today: (client: Moments) => ({
    queryKey: momentKeys.today,
    queryFn: ({ signal }: { signal: AbortSignal }) => client().getToday({}, { signal }),
    persister: journal,
    staleTime: 60_000,
  }),

  /** Friends' moments of their current day, newest first; locked until the caller posted. */
  friends: (client: Moments) => ({
    queryKey: momentKeys.friends,
    queryFn: ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      client().listFriendMoments({ page: { cursor: pageParam, limit: 20 } }, { signal }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    persister: journal,
    staleTime: 30_000,
  }),

  /** Friends' moments with a place the caller may see; empty while locked. */
  map: (client: Moments) => ({
    queryKey: momentKeys.map,
    queryFn: ({ signal }: { signal: AbortSignal }) => client().listMapMoments({}, { signal }),
    persister: journal,
    staleTime: 30_000,
  }),

  moment: (client: Moments, momentId: string) => ({
    queryKey: momentKeys.moment(momentId),
    queryFn: async ({ signal }: { signal: AbortSignal }) =>
      (await client().getMoment({ momentId }, { signal })).moment,
    persister: journal,
  }),

  /** The caller's own moments of a month (`month` 1–12), one per day. */
  calendar: (client: Moments, year: number, month: number) => ({
    queryKey: momentKeys.calendar(year, month),
    queryFn: () => client().getCalendar({ year, month }),
  }),

  settings: (client: Moments) => ({
    queryKey: momentKeys.settings,
    queryFn: async () => (await client().getMomentSettings({})).settings,
  }),

  /** Who reacted to a moment, newest first. */
  reactions: (client: Reactions, momentId: string) => ({
    queryKey: momentKeys.reactions(momentId),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listReactions({ momentId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),

  /** The caller's RealMoji library: at most one selfie per face. */
  realmojis: (client: Reactions) => ({
    queryKey: momentKeys.realmojis,
    queryFn: async () => (await client().listRealmojis({})).realmojis,
  }),
};
