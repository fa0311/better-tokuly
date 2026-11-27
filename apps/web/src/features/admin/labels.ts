import { admin, common } from "@tokuly/api";
import type { Tone } from "./ui/tone";

/**
 * Translation keys (in the `admin` namespace) for the API's enums. Unknown
 * values from a newer server fall back to the `UNSPECIFIED` label. The lists
 * the filter controls offer are in `filter-values.ts`.
 */
const name = (names: Record<number, string>, value: number) => names[value] ?? "UNSPECIFIED";

export const caseStatusKey = (value: admin.CaseStatus) =>
  `enums.caseStatus.${name(admin.CaseStatus, value)}`;
export const categoryKey = (value: admin.ReportCategory) =>
  `enums.category.${name(admin.ReportCategory, value)}`;
export const actionKey = (value: admin.ModerationActionKind) =>
  `enums.action.${name(admin.ModerationActionKind, value)}`;
export const subjectTypeKey = (value: common.SubjectType) =>
  `enums.subjectType.${name(common.SubjectType, value)}`;
export const accountStateKey = (value: admin.AccountState) =>
  `enums.accountState.${name(admin.AccountState, value)}`;
export const decisionKey = (value: admin.FlagDecision) =>
  `enums.decision.${name(admin.FlagDecision, value)}`;
export const announcementStatusKey = (value: admin.AnnouncementStatus) =>
  `enums.announcementStatus.${name(admin.AnnouncementStatus, value)}`;
export const roleKey = (role: string) => `enums.role.${role}`;

const ACTION_TONES: Partial<Record<admin.ModerationActionKind, Tone>> = {
  [admin.ModerationActionKind.REMOVE_CONTENT]: "warn",
  [admin.ModerationActionKind.WARN]: "warn",
  [admin.ModerationActionKind.SUSPEND]: "danger",
  [admin.ModerationActionKind.BAN]: "danger",
  [admin.ModerationActionKind.UNSUSPEND]: "good",
};

export const actionTone = (value: admin.ModerationActionKind): Tone =>
  ACTION_TONES[value] ?? "neutral";
