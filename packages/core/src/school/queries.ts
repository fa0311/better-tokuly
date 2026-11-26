import type { school } from "@tokuly/api";
import type { ClientSource } from "../query";
import { schoolKeys as keys } from "./keys";

type Timetable = ClientSource<typeof school.TimetableService>;
type Study = ClientSource<typeof school.StudyService>;
type Exams = ClientSource<typeof school.ExamService>;
type Assignments = ClientSource<typeof school.AssignmentService>;

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
  assignments: (client: Assignments, filter: school.AssignmentFilter) => ({
    queryKey: keys.assignments(filter),
    queryFn: () => client().listAssignments({ filter, page: { limit: 100 } }),
  }),
};
