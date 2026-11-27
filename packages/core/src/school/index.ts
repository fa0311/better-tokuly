/**
 * School logic every app shares: query keys and options, calendar dates and
 * times of day as the API writes them (`YYYY-MM-DD`, minutes after
 * midnight), which classes take place and what changed, the countdowns of
 * classes and exams, and "what is due". The timer, the screens and their
 * hooks stay in each app.
 */
export {
  addDays,
  clock,
  clockInput,
  dayLabel,
  daysBetween,
  fromLocalInput,
  isYmd,
  layoutWeekdays,
  momentLabel,
  mondayOf,
  parseClock,
  parseYmd,
  studyTime,
  timerText,
  toLocalInput,
  WEEKDAYS,
  weekdayOf,
  ymd,
} from "./dates";
export { refreshSchool, schoolKeys } from "./keys";
export {
  classColor,
  classStatusKey,
  classWhen,
  currentClass,
  type DueItem,
  dueItems,
  dueLeftOut,
  examCountdown,
  inkOn,
  lastCheerStillCounts,
  takesPlace,
  untilText,
  urgencyKey,
} from "./model";
export { type AssignmentPage, loadedAssignments, schoolQueries } from "./queries";
