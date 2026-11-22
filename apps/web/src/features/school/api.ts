import { type QueryClient, useQuery } from "@tanstack/react-query";
import { school, social } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

export const settingsClient = () => api().service(school.SchoolService);
export const timetableClient = () => api().service(school.TimetableService);
export const studyClient = () => api().service(school.StudyService);
export const roomClient = () => api().service(school.StudyRoomService);
export const examClient = () => api().service(school.ExamService);
export const assignmentClient = () => api().service(school.AssignmentService);
const friendClient = () => api().service(social.FriendService);

/** Shows why an action failed, in the school feature's own words where it has them. */
export function toastError(error: unknown): void {
  toast.error(errorMessage(error, { namespace: "school" }));
}

/**
 * Query keys. Everything starts with `school`, then the part of the feature,
 * so one part can be refreshed without touching the others.
 */
export const keys = {
  settings: ["school", "settings"] as const,
  subjects: (archived = false) => ["school", "subjects", { archived }] as const,
  presets: ["school", "presets"] as const,
  terms: ["school", "timetable", "terms"] as const,
  term: (termId: string) => ["school", "timetable", "term", termId] as const,
  day: (date: string, userId = "") => ["school", "timetable", "day", userId, date] as const,
  days: (start: string, end: string, userId = "") => ["school", "timetable", "days", userId, start, end] as const,
  now: (userId = "") => ["school", "timetable", "now", userId] as const,
  friendTimetables: ["school", "timetable", "friends"] as const,
  friendTimetable: (userId: string, termId = "") => ["school", "timetable", "friend", userId, termId] as const,
  free: (start: string, userIds: string[]) => ["school", "timetable", "free", start, userIds] as const,
  progress: ["school", "study", "progress"] as const,
  summary: (period: school.StatsPeriod, anchor = "") => ["school", "study", "summary", period, anchor] as const,
  heatmap: (userId = "") => ["school", "study", "heatmap", userId] as const,
  sessions: ["school", "study", "sessions"] as const,
  cheers: (sessionId: string) => ["school", "study", "cheers", sessionId] as const,
  studying: ["school", "studying"] as const,
  ranking: (anchor = "") => ["school", "study", "ranking", anchor] as const,
  rooms: (scope: school.RoomScope) => ["school", "rooms", "list", scope] as const,
  room: (roomId: string) => ["school", "rooms", "room", roomId] as const,
  exams: ["school", "exams"] as const,
  exam: (examId: string) => ["school", "exams", examId] as const,
  assignments: (filter: school.AssignmentFilter) => ["school", "assignments", filter] as const,
};

/** Refreshes everything under the given parts: `refresh(queryClient, "timetable", "subjects")`. */
export function refresh(queryClient: QueryClient, ...parts: string[]): void {
  for (const part of parts) void queryClient.invalidateQueries({ queryKey: ["school", part] });
}

export function useSchoolSettings() {
  return useQuery({ queryKey: keys.settings, queryFn: async () => (await settingsClient().getSchoolSettings({})).settings });
}

/** The caller's subjects by name; archived ones only when asked for. */
export function useSubjects(includeArchived = false, enabled = true) {
  return useQuery({
    queryKey: keys.subjects(includeArchived),
    queryFn: async () => (await timetableClient().listSubjects({ includeArchived })).subjects,
    enabled,
  });
}

export function useTerms() {
  return useQuery({ queryKey: keys.terms, queryFn: async () => (await timetableClient().listTerms({})).terms });
}

export function useTerm(termId: string | undefined) {
  return useQuery({
    queryKey: keys.term(termId ?? ""),
    queryFn: () => timetableClient().getTerm({ termId: termId ?? "" }),
    enabled: Boolean(termId),
  });
}

/** The class in progress and the next one; asked again every minute while on screen. */
export function useClassNow(userId = "") {
  return useQuery({
    queryKey: keys.now(userId),
    queryFn: async () => (await timetableClient().getNow({ userId })).now,
    refetchInterval: 60_000,
  });
}

export function useProgress() {
  return useQuery({ queryKey: keys.progress, queryFn: async () => (await studyClient().getStudyProgress({})).progress });
}

export function useExams() {
  return useQuery({ queryKey: keys.exams, queryFn: async () => (await examClient().listExams({})).exams });
}

export function useAssignments(filter: school.AssignmentFilter) {
  return useQuery({ queryKey: keys.assignments(filter), queryFn: () => assignmentClient().listAssignments({ filter, page: { limit: 100 } }) });
}

/** The viewer's friends (the cache entry the friends screen and chat use too). */
export function useFriends(query = "") {
  return useQuery({
    queryKey: ["social", "friends", "list", query],
    queryFn: () => friendClient().listFriends({ query, page: { limit: 100 } }),
  });
}
