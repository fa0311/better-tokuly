import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { admin, toMillis } from "@tokuly/api";
import { BadgeCheck, Ban, ExternalLink, FlaskConical, Gavel, KeyRound, LogOut, MessageSquareWarning, ScrollText, Undo2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { userAdminClient } from "../api";
import { formatDateTime } from "../format";
import { ActionList } from "../moderation/action-list";
import { ModerationRecord } from "../moderation/parts";
import { useStaffAccess } from "../roles";
import { ConsolePage, Fact, Section } from "../ui/console-page";
import { CopyableId } from "../ui/copy";
import { ErrorState } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { UserCell } from "../ui/user";
import { AccountStateChip, RoleChips } from "./parts";
import { RevokeSessionsDialog, RolesDialog, SuspendDialog, UnsuspendDialog, type UserAction, VerifyDialog, WarnDialog } from "./user-dialogs";

/** `/admin/users/:userId`: an account as staff see it, and everything staff can do to it. */
export function UserScreen({ userId }: { userId: string }) {
  const { t } = useTranslation("admin");
  const queryClient = useQueryClient();
  const key = ["admin", "user", userId];
  const detail = useQuery({ queryKey: key, queryFn: () => userAdminClient().getUser({ userId }) });
  const [action, setAction] = useState<UserAction | null>(null);

  /** Shows the account's new state at once, then re-reads everything that mentions it. */
  const applied = (account?: admin.AdminUser) => {
    if (account) queryClient.setQueryData<admin.GetUserResponse>(key, (previous) => (previous ? { ...previous, user: account } : previous));
    void queryClient.invalidateQueries({ queryKey: ["admin"] });
  };

  if (detail.isPending) {
    return (
      <ConsolePage title={t("users.title")} back="/admin/users">
        <div className="flex flex-col gap-4" aria-busy>
          <Skeleton className="h-32" />
          <Skeleton className="h-64" />
        </div>
      </ConsolePage>
    );
  }
  const account = detail.data?.user;
  const user = account?.user;
  if (!detail.data || !account || !user) {
    return (
      <ConsolePage title={t("users.title")} back="/admin/users">
        {detail.isError ? <ErrorState error={detail.error} onRetry={() => detail.refetch()} /> : <EmptyState emoji="🧭" title={t("users.notFound")} />}
      </ConsolePage>
    );
  }

  const dialog = (name: UserAction) => ({ account, open: action === name, onOpenChange: (open: boolean) => setAction(open ? name : null), onDone: applied });

  return (
    <ConsolePage title={user.deleted ? t("common.deletedUser") : `@${user.handle}`} back="/admin/users">
      <div className="@container">
        <div className="grid items-start gap-4 @4xl:grid-cols-[minmax(0,1fr)_22rem]">
          <div className="flex min-w-0 flex-col gap-4">
            <Identity account={account} />
            {account.suspension && <SuspensionCard suspension={account.suspension} onLift={() => setAction("unsuspend")} />}
            <Section
              title={t("users.record.title")}
              aside={
                <Link to="/admin/moderation" search={{ owner: user.id }} className="text-xs font-bold text-brand hover:underline">
                  {t("users.record.cases")}
                </Link>
              }
            >
              <div className="p-4">{detail.data.moderation ? <ModerationRecord record={detail.data.moderation} /> : <span className="text-sm text-ink-faint">—</span>}</div>
            </Section>
            <Section
              title={t("users.recentActions.title")}
              aside={
                <Link to="/admin/moderation/log" search={{ user: user.id }} className="text-xs font-bold text-brand hover:underline">
                  {t("users.recentActions.all")}
                </Link>
              }
            >
              <ActionList actions={detail.data.recentActions} showCase empty={t("users.recentActions.empty")} />
            </Section>
          </div>
          <div className="flex min-w-0 flex-col gap-4">
            <Actions account={account} onAction={setAction} />
            <Section title={t("users.facts.title")}>
              <dl className="divide-y divide-line">
                <Fact label="ID">
                  <CopyableId value={user.id} />
                </Fact>
                <Fact label={t("users.columns.email")}>
                  <span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span className="break-all">{account.email || "—"}</span>
                    {account.email && <ToneChip tone={account.emailVerified ? "good" : "neutral"}>{t(account.emailVerified ? "users.emailVerified" : "users.emailUnverified")}</ToneChip>}
                  </span>
                </Fact>
                <Fact label={t("users.facts.locale")}>{account.locale || "—"}</Fact>
                <Fact label={t("users.columns.created")}>{formatDateTime(account.createdAt)}</Fact>
                <Fact label={t("users.columns.lastActive")}>{account.lastActiveAt ? `${compactAge(toMillis(account.lastActiveAt))} (${formatDateTime(account.lastActiveAt)})` : "—"}</Fact>
                <Fact label={t("users.columns.sessions")}>{t("users.facts.sessionCount", { count: account.activeSessionCount })}</Fact>
                {account.deletionScheduledAt && <Fact label={t("users.facts.deletionAt")}>{formatDateTime(account.deletionScheduledAt)}</Fact>}
                {account.deletedAt && <Fact label={t("users.facts.deletedAt")}>{formatDateTime(account.deletedAt)}</Fact>}
              </dl>
            </Section>
          </div>
        </div>
      </div>

      <WarnDialog {...dialog("warn")} />
      <SuspendDialog {...dialog("suspend")} />
      <UnsuspendDialog {...dialog("unsuspend")} />
      <VerifyDialog {...dialog("verify")} />
      <RolesDialog {...dialog("roles")} />
      <RevokeSessionsDialog {...dialog("revoke")} />
    </ConsolePage>
  );
}

function Identity({ account }: { account: admin.AdminUser }) {
  const { t } = useTranslation("admin");
  const user = account.user;
  if (!user) return null;
  return (
    <Section>
      <div className="flex flex-wrap items-center gap-4 p-4">
        <Avatar user={user} size="lg" outlined />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-xl font-black">
            <span className="truncate">{user.deleted ? t("common.deletedUser") : user.displayName || user.handle}</span>
            {user.verified && <BadgeCheck className="size-5 shrink-0 text-brand" aria-label={t("users.verified")} />}
          </p>
          {!user.deleted && <p className="truncate text-sm font-bold text-ink-soft">@{user.handle}</p>}
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <AccountStateChip account={account} />
            <RoleChips roles={account.roles} emptyLabel="" />
          </div>
        </div>
        {!user.deleted && (
          <Link to="/u/$handle" params={{ handle: user.handle }} className="inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-bold text-brand hover:bg-surface-2">
            <ExternalLink className="size-4" />
            {t("users.openProfile")}
          </Link>
        )}
      </div>
    </Section>
  );
}

function SuspensionCard({ suspension, onLift }: { suspension: admin.Suspension; onLift(): void }) {
  const { t } = useTranslation("admin");
  return (
    <Section title={t(suspension.permanent ? "users.suspension.banTitle" : "users.suspension.title")} className="border-danger">
      <dl className="divide-y divide-line">
        <Fact label={t("common.reasonShort")}>
          <span className="whitespace-pre-wrap">{suspension.reason || "—"}</span>
        </Fact>
        <Fact label={t("users.suspension.by")}>
          <UserCell user={suspension.moderator} fallback={t("users.suspension.outside")} />
        </Fact>
        <Fact label={t("users.suspension.since")}>{formatDateTime(suspension.startedAt)}</Fact>
        <Fact label={t("users.suspension.ends")}>{suspension.permanent || !suspension.expiresAt ? t("users.suspension.never") : formatDateTime(suspension.expiresAt)}</Fact>
        {suspension.caseId && (
          <Fact label={t("users.suspension.case")}>
            <Link to="/admin/moderation/$caseId" params={{ caseId: suspension.caseId }} className="font-bold text-brand hover:underline">
              {t("moderation.log.viewCase")}
            </Link>
          </Fact>
        )}
      </dl>
      <div className="border-t-2 border-line p-3">
        <Button size="sm" variant="lime" icon={<Undo2 className="size-4" />} onClick={onLift}>
          {t("users.unsuspend.open")}
        </Button>
      </div>
    </Section>
  );
}

/** Every staff action on the account. What the caller may not do is disabled, with the reason underneath. */
function Actions({ account, onAction }: { account: admin.AdminUser; onAction(action: UserAction): void }) {
  const { t } = useTranslation("admin");
  const access = useStaffAccess();
  const me = useMe();
  const user = account.user;
  const gone = account.state === admin.AccountState.DELETED;
  const self = user?.id === me?.user?.id;
  const suspended = account.state === admin.AccountState.SUSPENDED;
  // Moderators cannot sanction admins and moderators; admins can.
  const protectedStaff = !access.admin && account.roles.some((role) => role === "admin" || role === "moderator");
  const blocked = gone ? t("users.actions.goneHint") : self ? t("users.actions.selfHint") : protectedStaff ? t("users.actions.staffHint") : undefined;

  const item = (action: UserAction, icon: ReactNode, label: string, options: { danger?: boolean; disabled?: boolean } = {}) => (
    <Button key={action} size="sm" variant="secondary" block className={options.danger ? "justify-start text-danger" : "justify-start"} icon={icon} disabled={options.disabled} onClick={() => onAction(action)}>
      {label}
    </Button>
  );

  return (
    <Section title={t("users.actions.title")}>
      <div className="flex flex-col gap-2 p-4">
        {item("warn", <MessageSquareWarning className="size-4" />, t("users.warn.open"), { disabled: Boolean(blocked) })}
        {item("suspend", <Ban className="size-4" />, t(suspended ? "users.suspend.openReplace" : "users.suspend.open"), { danger: true, disabled: Boolean(blocked) })}
        {suspended && item("unsuspend", <Undo2 className="size-4" />, t("users.unsuspend.open"), { disabled: protectedStaff })}
        {item("verify", <BadgeCheck className="size-4" />, t(user?.verified ? "users.verify.removeOpen" : "users.verify.grantOpen"), { disabled: gone })}
        {item("revoke", <LogOut className="size-4" />, t("users.revoke.open"), { danger: true, disabled: gone || protectedStaff })}
        {access.admin && item("roles", <KeyRound className="size-4" />, t("users.roles.open"), { disabled: gone })}
        {blocked && <p className="px-1 text-xs text-ink-faint">{blocked}</p>}
      </div>
      {user && !gone && (
        <div className="flex flex-col border-t-2 border-line p-2 text-sm font-bold">
          <Link to="/admin/moderation" search={{ owner: user.id }} className="flex items-center gap-2 rounded-xl px-2 py-2 hover:bg-surface-2">
            <Gavel className="size-4 text-ink-soft" />
            {t("users.links.cases")}
          </Link>
          {access.admin && (
            <>
              <Link to="/admin/audit" search={{ targetUser: user.id }} className="flex items-center gap-2 rounded-xl px-2 py-2 hover:bg-surface-2">
                <ScrollText className="size-4 text-ink-soft" />
                {t("users.links.audit")}
              </Link>
              <Link to="/admin/flags" search={{ evaluate: user.id }} className="flex items-center gap-2 rounded-xl px-2 py-2 hover:bg-surface-2">
                <FlaskConical className="size-4 text-ink-soft" />
                {t("users.links.flags")}
              </Link>
            </>
          )}
        </div>
      )}
    </Section>
  );
}
