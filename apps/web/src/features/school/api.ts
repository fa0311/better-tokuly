import { useQuery } from "@tanstack/react-query";
import { school, social } from "@tokuly/api";
import { schoolKeys, schoolQueries } from "@tokuly/core/school";
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

export function useSchoolSettings() {
  return useQuery({
    queryKey: schoolKeys.settings,
    queryFn: async () => (await settingsClient().getSchoolSettings({})).settings,
  });
}

/** The caller's subjects by name; archived ones only when asked for. */
export function useSubjects(includeArchived = false, enabled = true) {
  return useQuery({
    queryKey: schoolKeys.subjects(includeArchived),
    queryFn: async () => (await timetableClient().listSubjects({ includeArchived })).subjects,
    enabled,
  });
}

export function useTerms() {
  return useQuery({
    queryKey: schoolKeys.terms,
    queryFn: async () => (await timetableClient().listTerms({})).terms,
  });
}

export function useTerm(termId: string | undefined) {
  return useQuery({
    queryKey: schoolKeys.term(termId ?? ""),
    queryFn: () => timetableClient().getTerm({ termId: termId ?? "" }),
    enabled: Boolean(termId),
  });
}

/** The class in progress and the next one; asked again every minute while on screen. */
export function useClassNow(userId = "") {
  return useQuery(schoolQueries.now(timetableClient, userId));
}

export function useProgress() {
  return useQuery(schoolQueries.progress(studyClient));
}

export function useExams() {
  return useQuery(schoolQueries.exams(examClient));
}

export function useAssignments(filter: school.AssignmentFilter) {
  return useQuery(schoolQueries.assignments(assignmentClient, filter));
}

/** The viewer's friends (the cache entry the friends screen and chat use too). */
export function useFriends(query = "") {
  return useQuery({
    queryKey: ["social", "friends", "list", query],
    queryFn: () => friendClient().listFriends({ query, page: { limit: 100 } }),
  });
}
