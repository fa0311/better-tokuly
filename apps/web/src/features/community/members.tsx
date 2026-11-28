import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { community, fromDate } from "@tokuly/api";
import {
  assignableRoles,
  can,
  communityKeys,
  ROLE_KEYS,
  refreshMembership,
  storeCommunity,
} from "@tokuly/core/community";
import {
  Ban,
  Clock,
  Crown,
  MessageCircle,
  MoreHorizontal,
  Search,
  Shield,
  TimerOff,
  User,
  UserMinus,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, ListRow, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { openDirectChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { communityClient, moderationClient, NS, useMembers } from "./api";
import { ReasonDialog } from "./mod-dialogs";
import { RoleChip, useDurationLabel, useWhen } from "./parts";

const R = community.MemberRole;
const P = community.Permission;

const ROLE_FILTERS = [R.UNSPECIFIED, R.OWNER, R.ADMIN, R.MODERATOR, R.MEMBER] as const;

/** Durations offered for timeouts and bans, in seconds (0 = for good). */
export const TIMEOUTS = [600, 3600, 86_400, 7 * 86_400, 28 * 86_400] as const;
export const BAN_LENGTHS = [0, 86_400, 7 * 86_400, 30 * 86_400] as const;

/** The member directory: staff first, searchable; the staff's actions on each member. */
export function MembersTab({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<community.MemberRole>(R.UNSPECIFIED);
  // Search a moment after typing stops.
  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(text.trim()), 250);
    return () => window.clearTimeout(timer);
  }, [text]);
  const members = useMembers(c.id, query, role);

  return (
    <div>
      <div className="flex flex-col gap-2 sm:flex-row">
        <TextField
          className="min-w-0 flex-1"
          aria-label={t("members.search")}
          placeholder={t("members.search")}
          value={text}
          maxLength={64}
          onChange={(event) => setText(event.target.value)}
          trailing={<Search className="mr-2.5 size-5 text-ink-faint" />}
        />
        <SelectField
          className="sm:w-44"
          value={String(role)}
          onChange={(next) => setRole(Number(next))}
          options={ROLE_FILTERS.map((value) => ({
            value: String(value),
            label: value === R.UNSPECIFIED ? t("members.everyone") : t(`role.${ROLE_KEYS[value]}`),
          }))}
        />
      </div>
      <div className="mt-3 rounded-card bg-surface p-1.5">
        {members.isPending ? (
          <Skeleton className="h-14" />
        ) : members.items?.length === 0 ? (
          <EmptyState emoji="🔍" title={t("members.none")} />
        ) : (
          <>
            {members.items?.map((member) => (
              <MemberRow key={member.user?.id} community={c} member={member} />
            ))}
            <LoadMore query={members} />
          </>
        )}
      </div>
    </div>
  );
}

function MemberRow({
  community: c,
  member,
}: {
  community: community.Community;
  member: community.Member;
}) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const durationLabel = useDurationLabel();
  const me = useMe();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const user = member.user;
  const userId = user?.id ?? "";
  const isSelf = me?.user?.id === userId;
  const viewerRole = c.viewer?.role ?? R.UNSPECIFIED;
  const below = viewerRole > member.role && !isSelf;
  const [dialog, setDialog] = useState<"remove" | "ban" | "timeout" | "transfer" | null>(null);
  const [seconds, setSeconds] = useState<number>(3600);
  const timeoutMs = member.timeoutUntil ? Number(member.timeoutUntil.seconds) * 1000 : 0;
  const timedOut = timeoutMs > Date.now();

  const refresh = () => {
    setDialog(null);
    void queryClient.invalidateQueries({ queryKey: communityKeys.memberLists(c.id) });
    void queryClient.invalidateQueries({ queryKey: communityKeys.communities });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: NS }));
  const setRole = useMutation({
    mutationFn: (role: community.MemberRole) =>
      communityClient().setMemberRole({ communityId: c.id, userId, role }),
    onSuccess: ({ member: next }) => {
      refresh();
      toast.success(
        t("members.roleDone", {
          name: user?.displayName,
          role: t(`role.${ROLE_KEYS[next?.role ?? R.MEMBER]}`),
        }),
      );
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (reason: string) =>
      moderationClient().removeMember({ communityId: c.id, userId, reason }),
    onSuccess: () => {
      refresh();
      toast(t("members.removeDone", { name: user?.displayName }));
    },
    onError,
  });
  const ban = useMutation({
    mutationFn: (reason: string) =>
      moderationClient().banMember({
        communityId: c.id,
        userId,
        reason,
        expiresAt: seconds > 0 ? fromDate(new Date(Date.now() + seconds * 1000)) : undefined,
      }),
    onSuccess: () => {
      refresh();
      void queryClient.invalidateQueries({ queryKey: communityKeys.bans(c.id) });
      toast(t("members.banDone", { name: user?.displayName }));
    },
    onError,
  });
  const timeout = useMutation({
    mutationFn: (reason: string) =>
      moderationClient().timeoutMember({
        communityId: c.id,
        userId,
        durationSeconds: seconds,
        reason,
      }),
    onSuccess: () => {
      refresh();
      toast(t("members.timeoutDone", { name: user?.displayName }));
    },
    onError,
  });
  const clearTimeout = useMutation({
    mutationFn: () => moderationClient().clearTimeout({ communityId: c.id, userId }),
    onSuccess: () => {
      refresh();
      toast(t("members.timeoutCleared", { name: user?.displayName }));
    },
    onError,
  });
  const transfer = useMutation({
    mutationFn: () => communityClient().transferOwnership({ communityId: c.id, userId }),
    onSuccess: ({ community: next }) => {
      if (next) storeCommunity(queryClient, next);
      refresh();
      refreshMembership(queryClient, c.id);
      toast.success(t("members.transferDone", { name: user?.displayName }));
    },
    onError,
  });

  const actions: MenuAction[] = [
    {
      label: t("members.profile"),
      icon: <User />,
      onSelect: () => user && void navigate({ to: "/u/$handle", params: { handle: user.handle } }),
    },
  ];
  if (!isSelf)
    actions.push({
      label: t("members.message"),
      icon: <MessageCircle />,
      onSelect: () => void openDirectChat(userId),
    });
  if (can(c, P.MANAGE_ROLES))
    for (const role of assignableRoles(viewerRole, member.role))
      actions.push({
        label: t("members.makeRole", { role: t(`role.${ROLE_KEYS[role]}`) }),
        icon: <Shield />,
        onSelect: () => setRole.mutate(role),
      });
  if (can(c, P.TRANSFER_OWNERSHIP) && !isSelf)
    actions.push({
      label: t("members.transfer"),
      icon: <Crown />,
      onSelect: () => setDialog("transfer"),
    });
  if (can(c, P.MANAGE_MEMBERS) && below) {
    actions.push(
      timedOut
        ? {
            label: t("members.clearTimeout"),
            icon: <TimerOff />,
            onSelect: () => clearTimeout.mutate(),
          }
        : {
            label: t("members.timeout"),
            icon: <Clock />,
            onSelect: () => {
              setSeconds(3600);
              setDialog("timeout");
            },
          },
      {
        label: t("members.remove"),
        icon: <UserMinus />,
        danger: true,
        onSelect: () => setDialog("remove"),
      },
      {
        label: t("members.ban"),
        icon: <Ban />,
        danger: true,
        onSelect: () => {
          setSeconds(0);
          setDialog("ban");
        },
      },
    );
  }

  const name = user?.displayName || user?.handle || "";
  return (
    <>
      <ListRow
        leading={<Avatar user={user} />}
        title={
          <span className="flex items-center gap-1.5">
            <span className="truncate">
              <EmojiText text={name} />
            </span>
            <RoleChip role={member.role} />
            {timedOut && (
              <Chip className="bg-sun text-[#17122b]">
                <Clock className="size-3" />
                {t("members.timedOut")}
              </Chip>
            )}
          </span>
        }
        subtitle={
          timedOut
            ? t("members.timedOutUntil", { time: when(timeoutMs) }) +
              (member.timeoutReason ? ` · ${member.timeoutReason}` : "")
            : `@${user?.handle}`
        }
        trailing={
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("members.more", { name })}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={actions}
          />
        }
      />
      <ReasonDialog
        open={dialog === "remove"}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t("members.removeTitle", { name })}
        description={t("members.removeBody")}
        confirmLabel={t("members.remove")}
        loading={remove.isPending}
        onConfirm={(reason) => remove.mutate(reason)}
      />
      <ReasonDialog
        open={dialog === "ban"}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t("members.banTitle", { name })}
        description={t("members.banBody")}
        confirmLabel={t("members.ban")}
        required
        loading={ban.isPending}
        onConfirm={(reason) => ban.mutate(reason)}
      >
        <SelectField
          label={t("members.length")}
          value={String(seconds)}
          onChange={(next) => setSeconds(Number(next))}
          options={BAN_LENGTHS.map((value) => ({
            value: String(value),
            label: durationLabel(value),
          }))}
        />
      </ReasonDialog>
      <ReasonDialog
        open={dialog === "timeout"}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t("members.timeoutTitle", { name })}
        description={t("members.timeoutBody")}
        confirmLabel={t("members.timeout")}
        danger={false}
        loading={timeout.isPending}
        onConfirm={(reason) => timeout.mutate(reason)}
      >
        <SelectField
          label={t("members.length")}
          value={String(seconds)}
          onChange={(next) => setSeconds(Number(next))}
          options={TIMEOUTS.map((value) => ({ value: String(value), label: durationLabel(value) }))}
        />
      </ReasonDialog>
      <ConfirmDialog
        open={dialog === "transfer"}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t("members.transferTitle", { name })}
        description={t("members.transferBody")}
        confirmLabel={t("members.transfer")}
        danger
        loading={transfer.isPending}
        onConfirm={() => transfer.mutate()}
      />
    </>
  );
}
