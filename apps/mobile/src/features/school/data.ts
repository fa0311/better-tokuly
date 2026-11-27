import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { school } from "@tokuly/api";
import { DEFAULT_TIME_ZONE, zonedTime } from "@tokuly/core/format";
import { loadedAssignments, schoolQueries } from "@tokuly/core/school";
import { useEffect, useState } from "react";
import { AppState } from "react-native";
import { currentAccount, isActive } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";

/**
 * School in the query cache. The keys, query options and rules are
 * `@tokuly/core/school`; this file wires them to the phone's API client.
 * Every client is `api()` when called: whoever is signed in now. The cache
 * starts over with every account switch, so nothing of one account's school
 * is ever shown to the next.
 */
export const timetableClient = () => api().service(school.TimetableService);
export const studyClient = () => api().service(school.StudyService);
export const examClient = () => api().service(school.ExamService);
export const assignmentClient = () => api().service(school.AssignmentService);

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "school";

/**
 * One write as the account signed in now (`send` is called at once, before
 * any await). Its answer — or its refusal — belongs to that account: once
 * another one took over it is `undefined`, so nothing of it is shown under
 * the next account (README, "Accounts").
 */
export async function asAccount<T>(send: () => Promise<T>): Promise<T | undefined> {
  const account = currentAccount();
  if (!account) return undefined;
  try {
    const answer = await send();
    return isActive(account) ? answer : undefined;
  } catch (error) {
    if (isActive(account)) throw error;
    return undefined;
  }
}

/** The signed-in account's time zone: the API's dates and "today" are on its calendar, not the phone's. */
export function useAccountTimeZone(): string {
  return useMe()?.timeZone || DEFAULT_TIME_ZONE;
}

/**
 * "Now" on the account's clock, again every `everyMs` and whenever the app
 * comes back to the front (a night in the background moves on to the new day).
 */
export function useAccountNow(everyMs = 30_000) {
  const zone = useAccountTimeZone();
  const [ms, setMs] = useState(Date.now);
  useEffect(() => {
    const update = () => setMs(Date.now());
    const timer = setInterval(update, everyMs);
    const appState = AppState.addEventListener("change", (state) => {
      if (state === "active") update();
    });
    return () => {
      clearInterval(timer);
      appState.remove();
    };
  }, [everyMs]);
  return { ms, zone, ...zonedTime(ms, zone) };
}

export function useClassNow() {
  return useQuery(schoolQueries.now(timetableClient));
}

export function useDay(date: string) {
  return useQuery(schoolQueries.day(timetableClient, date));
}

export function useWeek(monday: string, sunday: string) {
  return useQuery(schoolQueries.days(timetableClient, monday, sunday));
}

export function useExams() {
  return useQuery(schoolQueries.exams(examClient));
}

/** The caller's assignments as far as they are loaded, with the open counts; `paging` reads on. */
export function useAssignments(filter: school.AssignmentFilter) {
  const query = useInfiniteQuery(schoolQueries.assignments(assignmentClient, filter));
  return { ...query, data: loadedAssignments(query.data?.pages), paging: query };
}

export function useStudyingFriends() {
  return useQuery(schoolQueries.studying(studyClient));
}
