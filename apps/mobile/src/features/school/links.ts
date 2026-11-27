import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * School opens here: today (`/school`), the study timer (where its
 * reminder and a friend's profile card lead) and the assignments, where the
 * assignment reminders lead. Study rooms, exams, making the timetable,
 * friends' timetables and the settings stay on the web screens, signed in.
 */
export default [
  { path: /^\/school\/?$/, screen: () => "/school" },
  { path: /^\/school\/study\/?$/, screen: () => "/school/study" },
  { path: /^\/school\/assignments\/?$/, screen: () => "/school/assignments" },
  { path: /^\/school\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
