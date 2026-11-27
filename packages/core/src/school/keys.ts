import type { QueryClient } from "@tanstack/query-core";
import type { school } from "@tokuly/api";

/**
 * Query keys of the school domain. Everything starts with `school`, then the
 * part of the feature, so one part can be refreshed without touching the
 * others (`refreshSchool`).
 */
export const schoolKeys = {
  settings: ["school", "settings"] as const,
  subjects: (archived = false) => ["school", "subjects", { archived }] as const,
  presets: ["school", "presets"] as const,
  terms: ["school", "timetable", "terms"] as const,
  term: (termId: string) => ["school", "timetable", "term", termId] as const,
  day: (date: string, userId = "") => ["school", "timetable", "day", userId, date] as const,
  days: (start: string, end: string, userId = "") =>
    ["school", "timetable", "days", userId, start, end] as const,
  now: (userId = "") => ["school", "timetable", "now", userId] as const,
  friendTimetables: ["school", "timetable", "friends"] as const,
  friendTimetable: (userId: string, termId = "") =>
    ["school", "timetable", "friend", userId, termId] as const,
  free: (start: string, userIds: string[]) =>
    ["school", "timetable", "free", start, userIds] as const,
  progress: ["school", "study", "progress"] as const,
  summary: (period: school.StatsPeriod, anchor = "") =>
    ["school", "study", "summary", period, anchor] as const,
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

/** Refreshes everything under the given parts: `refreshSchool(queryClient, "timetable", "subjects")`. */
export function refreshSchool(queryClient: QueryClient, ...parts: string[]): void {
  for (const part of parts) void queryClient.invalidateQueries({ queryKey: ["school", part] });
}
