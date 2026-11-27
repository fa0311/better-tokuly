import type { school } from "@tokuly/api";
import type { ClientSource } from "../query";
import { schoolKeys as keys } from "./keys";

type Timetable = ClientSource<typeof school.TimetableService>;
type Study = ClientSource<typeof school.StudyService>;
type Exams = ClientSource<typeof school.ExamService>;
type Assignments = ClientSource<typeof school.AssignmentService>;

/** How many assignments a list asks for at a time. */
const ASSIGNMENT_PAGE_SIZE = 100;

/** One page of the caller's assignments, as it is cached. The counts are of all their open ones. */
export interface AssignmentPage {
  assignments: school.Assignment[];
  nextCursor: string;
  openCount: number;
  overdueCount: number;
}

/** The assignments loaded so far, with the counts of the first page; `undefined` until it is there. */
export function loadedAssignments(pages: readonly AssignmentPage[] | undefined) {
  const [first] = pages ?? [];
  if (!first) return undefined;
  return {
    assignments: (pages ?? []).flatMap((page) => page.assignments),
    openCount: first.openCount,
    overdueCount: first.overdueCount,
  };
}

/**
 * Query options of what the school screens read on every app. `userId`
 * empty is the caller; another user's timetable is only what they share
 * with the caller. Each app adds `enabled` and its hooks around them.
 */
export const schoolQueries = {
  /** The class in progress and the next one; asked again every minute while on screen. */
  now: (client: Timetable, userId = "") => ({
    queryKey: keys.now(userId),
    queryFn: async () => (await client().getNow({ userId })).now,
    refetchInterval: 60_000,
  }),
  /** One date resolved: its periods, the classes that take place, what changed. */
  day: (client: Timetable, date: string, userId = "") => ({
    queryKey: keys.day(date, userId),
    queryFn: async () => (await client().getDay({ date, userId })).day,
  }),
  /** The dates from `start` to `end` (both included), resolved. */
  days: (client: Timetable, start: string, end: string) => ({
    queryKey: keys.days(start, end),
    queryFn: async () => (await client().listDays({ startDate: start, endDate: end })).days,
  }),
  progress: (client: Study) => ({
    queryKey: keys.progress,
    queryFn: async () => (await client().getStudyProgress({})).progress,
  }),
  /** Friends studying right now, as the server lets the caller see them. */
  studying: (client: Study) => ({
    queryKey: keys.studying,
    queryFn: async () => (await client().listStudyingFriends({})).friends,
  }),
  exams: (client: Exams) => ({
    queryKey: keys.exams,
    queryFn: async () => (await client().listExams({})).exams,
  }),
  /** The caller's assignments by urgency, in pages. */
  assignments: (client: Assignments, filter: school.AssignmentFilter) => ({
    queryKey: keys.assignments(filter),
    queryFn: async ({ pageParam }: { pageParam: string }): Promise<AssignmentPage> => {
      const response = await client().listAssignments({
        filter,
        page: { limit: ASSIGNMENT_PAGE_SIZE, cursor: pageParam },
      });
      return {
        assignments: response.assignments,
        nextCursor: response.page?.nextCursor ?? "",
        openCount: response.openCount,
        overdueCount: response.overdueCount,
      };
    },
    initialPageParam: "",
    getNextPageParam: (page: AssignmentPage) => page.nextCursor || undefined,
  }),
};
