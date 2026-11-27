import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * School opens here: today (`/school`) and the assignments, where the
 * assignment reminders lead. The timer, study rooms, exams, making the
 * timetable, friends' timetables and the settings stay on the web screens,
 * signed in.
 */
export default [
  { path: /^\/school\/?$/, screen: () => "/school" },
  { path: /^\/school\/assignments\/?$/, screen: () => "/school/assignments" },
  { path: /^\/school\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
