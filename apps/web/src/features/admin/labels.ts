import { admin, common } from "@tokuly/api";
import type { Tone } from "./ui/tone";

/**
 * Translation keys (in the `admin` namespace) for the API's enums, and the
 * lists the filter controls offer. Unknown values from a newer server fall
 * back to the `UNSPECIFIED` label.
 */
const name = (names: Record<number, string>, value: number) => names[value] ?? "UNSPECIFIED";

export const caseStatusKey = (value: admin.CaseStatus) => `enums.caseStatus.${name(admin.CaseStatus, value)}`;
export const categoryKey = (value: admin.ReportCategory) => `enums.category.${name(admin.ReportCategory, value)}`;
export const actionKey = (value: admin.ModerationActionKind) => `enums.action.${name(admin.ModerationActionKind, value)}`;
export const subjectTypeKey = (value: common.SubjectType) => `enums.subjectType.${name(common.SubjectType, value)}`;
export const accountStateKey = (value: admin.AccountState) => `enums.accountState.${name(admin.AccountState, value)}`;
export const decisionKey = (value: admin.FlagDecision) => `enums.decision.${name(admin.FlagDecision, value)}`;
export const announcementStatusKey = (value: admin.AnnouncementStatus) => `enums.announcementStatus.${name(admin.AnnouncementStatus, value)}`;
export const roleKey = (role: string) => `enums.role.${role}`;

export const REPORT_CATEGORIES = [
  admin.ReportCategory.SPAM,
  admin.ReportCategory.HARASSMENT,
  admin.ReportCategory.HATE,
  admin.ReportCategory.SEXUAL,
  admin.ReportCategory.VIOLENCE,
  admin.ReportCategory.SELF_HARM,
  admin.ReportCategory.ILLEGAL,
  admin.ReportCategory.IMPERSONATION,
  admin.ReportCategory.COPYRIGHT,
  admin.ReportCategory.OTHER,
] as const;

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

/** Everything a staff member (or the system) can have done, for the log's filter. */
export const ACTION_KINDS = [
  admin.ModerationActionKind.DISMISS,
  admin.ModerationActionKind.REMOVE_CONTENT,
  admin.ModerationActionKind.WARN,
  admin.ModerationActionKind.SUSPEND,
  admin.ModerationActionKind.BAN,
  admin.ModerationActionKind.UNSUSPEND,
] as const;

const ACTION_TONES: Partial<Record<admin.ModerationActionKind, Tone>> = {
  [admin.ModerationActionKind.REMOVE_CONTENT]: "warn",
  [admin.ModerationActionKind.WARN]: "warn",
  [admin.ModerationActionKind.SUSPEND]: "danger",
  [admin.ModerationActionKind.BAN]: "danger",
  [admin.ModerationActionKind.UNSUSPEND]: "good",
};

export const actionTone = (value: admin.ModerationActionKind): Tone => ACTION_TONES[value] ?? "neutral";
