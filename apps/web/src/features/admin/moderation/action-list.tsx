import { Link } from "@tanstack/react-router";
import type { admin } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { formatDateTime } from "../format";
import { UserCell } from "../ui/user";
import { ActionChip } from "./parts";

/**
 * Moderation log entries as a compact timeline: used for a case's history
 * and an account's recent record. `showTarget` adds who the action was taken
 * against; `showCase` links the case it belongs to.
 */
export function ActionList({ actions, showTarget, showCase, empty }: { actions: admin.ModerationAction[]; showTarget?: boolean; showCase?: boolean; empty: string }) {
  const { t } = useTranslation("admin");
  if (actions.length === 0) return <p className="px-4 py-4 text-sm text-ink-faint">{empty}</p>;
  return (
    <ol className="divide-y divide-line">
      {actions.map((action) => (
        <li key={action.id} className="flex flex-col gap-1.5 px-4 py-3 text-sm">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <ActionChip kind={action.kind} />
            <span className="text-xs text-ink-soft">{formatDateTime(action.createdAt)}</span>
            {showCase && action.caseId && (
              <Link to="/admin/moderation/$caseId" params={{ caseId: action.caseId }} className="text-xs font-bold text-brand hover:underline">
                {t("moderation.log.viewCase")}
              </Link>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-soft">
            <span className="inline-flex items-center gap-1.5">
              {t("moderation.log.by")}
              <UserCell user={action.moderator} fallback={t("common.system")} />
            </span>
            {showTarget && action.targetUser && (
              <span className="inline-flex items-center gap-1.5">
                {t("moderation.log.against")}
                <UserCell user={action.targetUser} />
              </span>
            )}
          </div>
          {action.reason && <p className="whitespace-pre-wrap break-words">{action.reason}</p>}
          {action.note && (
            <p className="whitespace-pre-wrap break-words rounded-lg bg-surface-2 px-2.5 py-1.5 text-xs text-ink-soft">
              <span className="font-bold">{t("common.note")}: </span>
              {action.note}
            </p>
          )}
          {action.expiresAt && <p className="text-xs text-ink-soft">{t("moderation.log.until", { time: formatDateTime(action.expiresAt) })}</p>}
        </li>
      ))}
    </ol>
  );
}
