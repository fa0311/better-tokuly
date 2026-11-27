import { type admin, common } from "@tokuly/api";

/**
 * The enum values the console's filters offer and its addresses accept. The
 * admin ones are plain numbers, every value of the enum but `UNSPECIFIED` in
 * the enum's order, so a route reading its address loads none of the admin
 * API code; `filter-values.test.ts` holds them to the enums.
 */

/** `admin.ReportCategory`: SPAM … OTHER. */
export const REPORT_CATEGORIES = [
  1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
] as const satisfies readonly admin.ReportCategory[];

/** `admin.ModerationActionKind`: everything a staff member (or the system) can have done. */
export const ACTION_KINDS = [
  1, 2, 3, 4, 5, 6,
] as const satisfies readonly admin.ModerationActionKind[];

/** `admin.AccountState`: ACTIVE … DELETED. */
export const ACCOUNT_STATES = [1, 2, 3, 4] as const satisfies readonly admin.AccountState[];

/** Everything that can be reported, in the order of the API's enum. */
export const SUBJECT_TYPES = [
  common.SubjectType.USER,
  common.SubjectType.CHANNEL,
  common.SubjectType.NOTE,
  common.SubjectType.STORY,
  common.SubjectType.VIDEO,
  common.SubjectType.SHORT,
  common.SubjectType.LIVE_STREAM,
  common.SubjectType.CLIP,
  common.SubjectType.PLAYLIST,
  common.SubjectType.COMMENT,
  common.SubjectType.CHAT_MESSAGE,
  common.SubjectType.LIVE_CHAT_MESSAGE,
  common.SubjectType.STAMP_PACK,
  common.SubjectType.STAMP,
  common.SubjectType.PLACE,
  common.SubjectType.NEWS_ARTICLE,
] as const;
