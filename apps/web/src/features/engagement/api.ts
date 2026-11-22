import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { common, engagement } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import {
  type Engagement,
  reactionFields,
  type Subject,
  sameReaction,
  subjectKey,
  toEngagement,
  type ViewerReaction,
  withCounters,
  withViewerReaction,
} from "./model";

export const reactionClient = () => api().service(engagement.ReactionService);
export const bookmarkClient = () => api().service(engagement.BookmarkService);
export const engagementClient = () => api().service(engagement.EngagementService);
export const reportClient = () => api().service(engagement.ReportService);

/** Query keys of the engagement domain. */
export const engagementKeys = {
  all: ["engagement"] as const,
  subjects: ["engagement", "subject"] as const,
  subject: (subject: Subject) => ["engagement", "subject", subject.type, subject.id] as const,
  reactors: (subject: Subject, emoji: string) =>
    ["engagement", "reactors", subject.type, subject.id, emoji] as const,
  myReactions: ["engagement", "my-reactions"] as const,
  collections: ["engagement", "collections"] as const,
  collectionsFor: (subject: Subject) =>
    ["engagement", "collections", "for", subject.type, subject.id] as const,
  bookmarks: ["engagement", "bookmarks"] as const,
  bookmarkList: (collectionId: string) => ["engagement", "bookmarks", collectionId] as const,
  reports: ["engagement", "reports"] as const,
  shareStats: (subject: Subject) =>
    ["engagement", "share-stats", subject.type, subject.id] as const,
};

// ───────────────────────────── Batched loading ─────────────────────────────

/** `BatchGetEngagement` takes at most this many subjects. */
const BATCH_LIMIT = 100;

interface Waiter {
  subject: Subject;
  resolve(value: Engagement | null): void;
  reject(error: unknown): void;
}

let waiting: Waiter[] = [];
let scheduled = false;

/** Loads one batch (subjects of one type, at most `BATCH_LIMIT`) and answers everyone waiting for one of them. */
async function loadChunk(chunk: Subject[], waiters: Waiter[]): Promise<void> {
  const keys = new Set(chunk.map(subjectKey));
  const mine = waiters.filter((waiter) => keys.has(subjectKey(waiter.subject)));
  try {
    const response = await engagementClient().batchGetEngagement({ subjects: chunk });
    const found = new Map<string, Engagement>();
    for (const item of response.engagements) {
      const type = item.subject?.type ?? common.SubjectType.UNSPECIFIED;
      // Answers are matched by the ID as it was asked (a short ID stays a short ID).
      found.set(`${type}:${item.requestedId || item.subject?.id}`, toEngagement(item));
    }
    // A subject missing from the answer is one the caller cannot see.
    for (const waiter of mine) waiter.resolve(found.get(subjectKey(waiter.subject)) ?? null);
  } catch (error) {
    for (const waiter of mine) waiter.reject(error);
  }
}

async function flush(): Promise<void> {
  scheduled = false;
  const waiters = waiting;
  waiting = [];
  const subjects = [
    ...new Map(waiters.map((waiter) => [subjectKey(waiter.subject), waiter.subject])).values(),
  ];
  // One request per subject type: a batch is refused as a whole when it names a type no
  // domain serves (yet), and that must not take the notes next to it down as well.
  const byType = new Map<common.SubjectType, Subject[]>();
  for (const subject of subjects)
    byType.set(subject.type, [...(byType.get(subject.type) ?? []), subject]);
  const chunks = [...byType.values()].flatMap((group) =>
    Array.from({ length: Math.ceil(group.length / BATCH_LIMIT) }, (_, index) =>
      group.slice(index * BATCH_LIMIT, (index + 1) * BATCH_LIMIT),
    ),
  );
  await Promise.all(chunks.map((chunk) => loadChunk(chunk, waiters)));
}

/**
 * The engagement of one subject. Calls made in the same moment — every card
 * of a list rendering — travel together: one `BatchGetEngagement` request
 * per subject type. `null` when the subject is gone or hidden from the caller.
 */
export function loadEngagement(subject: Subject): Promise<Engagement | null> {
  return new Promise((resolve, reject) => {
    waiting.push({ subject, resolve, reject });
    if (scheduled) return;
    scheduled = true;
    setTimeout(() => void flush(), 10);
  });
}

/**
 * Counts and the caller's own reaction and bookmark for a subject. Lists need
 * nothing special: their rows each call this and share one request. The
 * numbers stay live while a control using `useSubjectLive` is on screen.
 */
export function useEngagement(subject: Subject | undefined) {
  return useQuery({
    queryKey: subject
      ? engagementKeys.subject(subject)
      : (["engagement", "subject", "none"] as const),
    queryFn: () => loadEngagement(subject!),
    enabled: Boolean(subject?.id),
    staleTime: 60_000,
  });
}

/** Loads the engagement of many subjects ahead of their controls rendering (one request per 100). */
export function prefetchEngagement(queryClient: QueryClient, subjects: Subject[]): Promise<void> {
  return Promise.all(
    subjects.map((subject) =>
      queryClient.prefetchQuery({
        queryKey: engagementKeys.subject(subject),
        queryFn: () => loadEngagement(subject),
        staleTime: 60_000,
      }),
    ),
  ).then(() => undefined);
}

function patch(
  queryClient: QueryClient,
  subject: Subject,
  update: (current: Engagement) => Engagement,
): void {
  queryClient.setQueryData<Engagement | null>(engagementKeys.subject(subject), (current) =>
    current ? update(current) : current,
  );
}

/** A realtime `counters_changed`: applied to every cached entry of that subject (it may be cached under its short ID too). */
export function applyCounters(
  queryClient: QueryClient,
  type: common.SubjectType,
  uuid: string,
  counters: engagement.CountersChanged,
): void {
  queryClient.setQueriesData<Engagement | null>(
    { queryKey: [...engagementKeys.subjects, type] },
    (current) =>
      current && current.subjectId === uuid ? withCounters(current, counters) : current,
  );
}

// ───────────────────────────── Reactions ─────────────────────────────

/**
 * Sets, changes or takes back (`undefined`) the caller's reaction. The
 * controls show the result at once; the server's totals replace the guess
 * when they arrive, and a refusal puts things back with a toast.
 */
export function useReact(subject: Subject): (next: ViewerReaction | undefined) => Promise<void> {
  const queryClient = useQueryClient();
  return async (next) => {
    const key = engagementKeys.subject(subject);
    await queryClient.cancelQueries({ queryKey: key, exact: true });
    const before = queryClient.getQueryData<Engagement | null>(key);
    if (before && sameReaction(before.viewerReaction, next)) return;
    patch(queryClient, subject, (current) => withViewerReaction(current, next));
    try {
      const response = next
        ? await reactionClient().setReaction({ subject, reaction: next })
        : await reactionClient().removeReaction({ subject });
      patch(queryClient, subject, (current) => ({
        ...current,
        ...reactionFields(response.reactions),
      }));
      void queryClient.invalidateQueries({
        queryKey: ["engagement", "reactors", subject.type, subject.id],
      });
      void queryClient.invalidateQueries({ queryKey: engagementKeys.myReactions });
    } catch (error) {
      queryClient.setQueryData(key, before);
      void queryClient.invalidateQueries({ queryKey: key, exact: true });
      toast.error(errorMessage(error));
    }
  };
}

/** Who reacted to a subject, most recent first; `emoji` narrows to one emoji. */
export function useReactors(subject: Subject, emoji: string, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: engagementKeys.reactors(subject, emoji),
    queryFn: ({ pageParam }) =>
      reactionClient().listReactions({
        subject,
        kind: emoji ? engagement.ReactionKind.EMOJI : engagement.ReactionKind.UNSPECIFIED,
        emoji,
        page: { cursor: pageParam, limit: 30 },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

/** Subjects the caller reacted to, most recent first. */
export function useMyReactions() {
  return useInfiniteQuery({
    queryKey: engagementKeys.myReactions,
    queryFn: ({ pageParam }) =>
      reactionClient().listMyReactions({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

// ───────────────────────────── Bookmarks ─────────────────────────────

/** The caller's collections, the default one ("saved") first. */
export function useCollections() {
  return useQuery({
    queryKey: engagementKeys.collections,
    queryFn: async () => (await bookmarkClient().listCollections({})).collections,
  });
}

/** The caller's collections with `containsSubject` filled in for `subject`: the collection chooser. */
export function useCollectionsFor(subject: Subject, enabled: boolean) {
  return useQuery({
    queryKey: engagementKeys.collectionsFor(subject),
    queryFn: async () => (await bookmarkClient().listCollections({ subject })).collections,
    enabled,
    // Always the truth when the chooser opens: saving elsewhere changes it.
    staleTime: 0,
  });
}

/** Saved subjects of a collection (`""` = everything saved), most recently saved first. */
export function useBookmarks(collectionId: string) {
  return useInfiniteQuery({
    queryKey: engagementKeys.bookmarkList(collectionId),
    queryFn: ({ pageParam }) =>
      bookmarkClient().listBookmarks({ collectionId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** Everything that lists saved subjects or collections is re-read. */
export function refreshBookmarks(queryClient: QueryClient): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: engagementKeys.collections }),
    queryClient.invalidateQueries({ queryKey: engagementKeys.bookmarks }),
  ]);
}

/**
 * Saves a subject (into `collectionId` and the default collection, or only
 * the default one) or removes it (from `collectionId`, or from everywhere
 * when none is named). Returns whether the subject is saved afterwards;
 * `undefined` when the server refused (a toast says why).
 */
export function useBookmark(subject: Subject): {
  save(collectionId?: string): Promise<boolean | undefined>;
  remove(collectionId?: string): Promise<boolean | undefined>;
} {
  const queryClient = useQueryClient();
  const key = engagementKeys.subject(subject);

  const run = async (
    optimistic: boolean | undefined,
    call: () => Promise<{ bookmarked: boolean; bookmarkCount: bigint }>,
  ) => {
    await queryClient.cancelQueries({ queryKey: key, exact: true });
    const before = queryClient.getQueryData<Engagement | null>(key);
    if (optimistic !== undefined) {
      patch(queryClient, subject, (current) =>
        current.bookmarked === optimistic
          ? current
          : {
              ...current,
              bookmarked: optimistic,
              bookmarkCount: Math.max(0, current.bookmarkCount + (optimistic ? 1 : -1)),
            },
      );
    }
    try {
      const result = await call();
      patch(queryClient, subject, (current) => ({
        ...current,
        bookmarked: result.bookmarked,
        bookmarkCount: Number(result.bookmarkCount),
      }));
      void refreshBookmarks(queryClient);
      return result.bookmarked;
    } catch (error) {
      queryClient.setQueryData(key, before);
      void queryClient.invalidateQueries({ queryKey: key, exact: true });
      toast.error(errorMessage(error));
      return undefined;
    }
  };

  return {
    save: (collectionId = "") =>
      run(true, async () => {
        const response = await bookmarkClient().addBookmark({ subject, collectionId });
        return { bookmarked: true, bookmarkCount: response.bookmarkCount };
      }),
    // Leaving one named collection keeps the subject saved; only "everywhere" is shown ahead of the answer.
    remove: (collectionId = "") =>
      run(collectionId ? undefined : false, () =>
        bookmarkClient().removeBookmark({ subject, collectionId }),
      ),
  };
}

// ───────────────────────────── Shares ─────────────────────────────

/** Records a share and returns the link to hand out. The share counter follows. */
export async function recordShare(
  queryClient: QueryClient,
  subject: Subject,
  target: engagement.ShareTarget,
): Promise<string> {
  const response = await engagementClient().recordShare({ subject, target });
  patch(queryClient, subject, (current) => ({
    ...current,
    shareCount: Number(response.shareCount),
  }));
  void queryClient.invalidateQueries({ queryKey: engagementKeys.shareStats(subject) });
  return response.url;
}

/** How often a subject was shared, by target. Only its owner may ask. */
export function useShareStats(subject: Subject, enabled: boolean) {
  return useQuery({
    queryKey: engagementKeys.shareStats(subject),
    queryFn: () => engagementClient().getShareStats({ subject }),
    enabled,
  });
}

// ───────────────────────────── Reports ─────────────────────────────

/** The caller's reports, newest first. */
export function useMyReports() {
  return useInfiniteQuery({
    queryKey: engagementKeys.reports,
    queryFn: ({ pageParam }) =>
      reportClient().listMyReports({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}
