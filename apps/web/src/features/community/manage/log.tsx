import { community } from "@tokuly/api";
import { can, pseudonymName } from "@tokuly/core/community";
import { History } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { NS, useModerationLog } from "../api";
import { ContentHistoryDialog } from "../mod-dialogs";
import { useWhen } from "../parts";

const M = community.ModerationAction;

/** Keys of the log's actions in the strings (`log.action.<key>`). */
const ACTION_KEYS: Record<community.ModerationAction, string> = {
  [M.UNSPECIFIED]: "other",
  [M.MEMBER_REMOVED]: "memberRemoved",
  [M.MEMBER_BANNED]: "memberBanned",
  [M.MEMBER_UNBANNED]: "memberUnbanned",
  [M.MEMBER_TIMED_OUT]: "memberTimedOut",
  [M.TIMEOUT_CLEARED]: "timeoutCleared",
  [M.ROLE_CHANGED]: "roleChanged",
  [M.OWNERSHIP_TRANSFERRED]: "ownershipTransferred",
  [M.JOIN_REQUEST_APPROVED]: "joinRequestApproved",
  [M.JOIN_REQUEST_DECLINED]: "joinRequestDeclined",
  [M.THREAD_DELETED]: "threadDeleted",
  [M.REPLY_DELETED]: "replyDeleted",
  [M.POST_HIDDEN]: "postHidden",
  [M.POST_RESTORED]: "postRestored",
  [M.POST_REMOVED]: "postRemoved",
  [M.THREAD_PINNED]: "threadPinned",
  [M.THREAD_UNPINNED]: "threadUnpinned",
  [M.THREAD_LOCKED]: "threadLocked",
  [M.THREAD_UNLOCKED]: "threadUnlocked",
  [M.ANNOUNCEMENT_SET]: "announcementSet",
  [M.ANNOUNCEMENT_CLEARED]: "announcementCleared",
  [M.AUTHOR_REVEALED]: "authorRevealed",
  [M.SAFETY_SETTINGS_CHANGED]: "safetySettingsChanged",
  [M.SLOW_MODE_CHANGED]: "slowModeChanged",
  [M.CHANNEL_CREATED]: "channelCreated",
  [M.CHANNEL_UPDATED]: "channelUpdated",
  [M.CHANNEL_DELETED]: "channelDeleted",
  [M.COMMUNITY_UPDATED]: "communityUpdated",
  [M.CONTENT_TAKEN_DOWN]: "contentTakenDown",
  [M.EVENT_REMOVED]: "eventRemoved",
  [M.INVITE_REVOKED]: "inviteRevoked",
};

/** The moderation log, newest first: who did what to whom (never who wrote an anonymous post). */
export function LogSection({ community: c }: { community: community.Community }) {
  const { t, i18n } = useTranslation(NS);
  const when = useWhen();
  const [action, setAction] = useState<community.ModerationAction>(M.UNSPECIFIED);
  const [history, setHistory] = useState<community.ContentRef["content"]>();
  const log = useModerationLog(c.id, action);
  const canRead = can(c, community.Permission.MODERATE_CONTENT);
  const actions = Object.keys(ACTION_KEYS)
    .map(Number)
    .filter((value) => value !== M.UNSPECIFIED) as community.ModerationAction[];
  return (
    <div className="flex flex-col gap-3">
      <SelectField
        label={t("log.filter")}
        value={String(action)}
        onChange={(next) => setAction(Number(next))}
        options={[
          { value: String(M.UNSPECIFIED), label: t("log.all") },
          ...actions.map((value) => ({
            value: String(value),
            label: t(`log.action.${ACTION_KEYS[value]}`),
          })),
        ]}
      />
      {log.isPending ? (
        <Skeleton className="h-40" />
      ) : log.items?.length === 0 ? (
        <EmptyState emoji="📒" title={t("log.empty")} />
      ) : (
        <ol className="flex flex-col gap-2">
          {log.items?.map((entry) => {
            const content: community.ContentRef["content"] = entry.replyId
              ? { case: "replyId", value: entry.replyId }
              : entry.threadId
                ? { case: "threadId", value: entry.threadId }
                : entry.eventId
                  ? { case: "eventId", value: entry.eventId }
                  : { case: undefined };
            const target = entry.targetPseudonym
              ? pseudonymName(entry.targetPseudonym, i18n.language)
              : entry.targetUser
                ? `@${entry.targetUser.handle}`
                : "";
            const details = Object.entries(entry.details);
            return (
              <li key={entry.id} className="rounded-card bg-surface p-3.5">
                <p className="font-bold">
                  {t(`log.action.${ACTION_KEYS[entry.action] ?? "other"}`)}
                </p>
                <p className="mt-0.5 text-sm text-ink-soft">
                  {entry.actor
                    ? entry.siteModeration
                      ? t("log.bySite", { name: entry.actor.displayName || entry.actor.handle })
                      : t("log.by", { name: entry.actor.displayName || entry.actor.handle })
                    : t("log.automatic")}
                  {target && ` → ${target}`}
                </p>
                {entry.reason && (
                  <p className="mt-1 whitespace-pre-wrap break-words text-sm">
                    {t("log.reason", { reason: entry.reason })}
                  </p>
                )}
                {details.length > 0 && (
                  <p className="mt-1 break-words text-xs text-ink-faint">
                    {details.map(([key, value]) => `${key}: ${value}`).join(" · ")}
                  </p>
                )}
                <div className="mt-1 flex items-center gap-2">
                  <time className="flex-1 text-xs text-ink-faint">
                    {when(Number(entry.createdAt?.seconds ?? 0n) * 1000)}
                  </time>
                  {canRead && content.case && (
                    <Button
                      variant="ghost"
                      size="sm"
                      icon={<History className="size-4" />}
                      onClick={() => setHistory(content)}
                    >
                      {t("history.action")}
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
          <LoadMore query={log} />
        </ol>
      )}
      <ContentHistoryDialog content={history} onClose={() => setHistory(undefined)} />
    </div>
  );
}
