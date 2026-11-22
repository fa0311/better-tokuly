import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { chat, type common } from "@tokuly/api";
import {
  Check,
  Crown,
  EyeOff,
  Images,
  LogOut,
  MoreHorizontal,
  Pencil,
  Pin,
  Search,
  Shield,
  ShieldOff,
  UserMinus,
  UserPlus,
  UserRound,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { useConversationActions } from "./actions";
import { chatKeys, conversationClient } from "./api";
import { ConversationAvatar } from "./conversation-avatar";
import { useConversationContext } from "./conversation-context";
import type { ConversationDialog } from "./conversation-dialogs";
import { putConversation } from "./conversations";
import { errorMessage } from "./errors";
import { FriendPicker } from "./friend-picker";
import { GroupIconPicker } from "./group-icon";
import { useMembers } from "./members";
import { isGroup, isMuted, manages, peerOf } from "./model";
import { SectionTitle } from "./parts";
import { displayName, titleOf } from "./people";

export type PanelKind = "info" | "search" | "pinned" | "media";

/**
 * Everything about the conversation itself: who it is with, the viewer's
 * preferences for it, and — for groups — its name, icon and members with
 * their roles.
 */
export function InfoPanel({
  onShow,
  onDialog,
}: {
  onShow(panel: PanelKind): void;
  onDialog(dialog: ConversationDialog): void;
}) {
  const { t } = useTranslation("chat");
  const { conversation, me } = useConversationContext();
  const actions = useConversationActions();
  const viewer = conversation.viewer;
  const group = isGroup(conversation);
  const peer = peerOf(conversation, me.id);
  const accepted = viewer?.requestState === chat.RequestState.ACCEPTED;

  return (
    <div className="flex flex-col gap-5">
      {group ? (
        <GroupIdentity />
      ) : (
        <div className="flex flex-col items-center gap-2 text-center">
          <ConversationAvatar conversation={conversation} myId={me.id} size="xl" />
          <p className="max-w-full truncate text-xl font-black">{titleOf(conversation, me.id)}</p>
          {peer && !peer.deleted && (
            <>
              <p className="-mt-1 font-bold text-ink-soft">@{peer.handle}</p>
              <Link
                to="/u/$handle"
                params={{ handle: peer.handle }}
                className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
              >
                <UserRound className="size-4" />
                {t("info.profile")}
              </Link>
            </>
          )}
        </div>
      )}

      <div className="grid grid-cols-3 gap-2">
        <Shortcut icon={<Search />} label={t("info.search")} onClick={() => onShow("search")} />
        <Shortcut icon={<Pin />} label={t("info.pins")} onClick={() => onShow("pinned")} />
        <Shortcut icon={<Images />} label={t("info.media")} onClick={() => onShow("media")} />
      </div>

      {accepted && viewer && (
        <section className="-mx-3">
          <SwitchRow
            label={t("info.pinned")}
            hint={t("info.pinnedHint")}
            checked={viewer.pinned}
            onChange={(pinned) => void actions.setPinned(conversation, pinned)}
          />
          <SwitchRow
            label={t("info.muted")}
            hint={t("info.mutedHint")}
            checked={isMuted(viewer)}
            onChange={(muted) =>
              muted ? onDialog({ type: "mute", conversation }) : void actions.unmute(conversation)
            }
          />
          <SwitchRow
            label={t("info.archived")}
            hint={t("info.archivedHint")}
            checked={viewer.archived}
            onChange={(archived) => void actions.setArchived(conversation, archived)}
          />
        </section>
      )}

      {group && <Members />}

      <section className="flex flex-col gap-2">
        <Button
          variant="ghost"
          className="justify-start"
          icon={<EyeOff className="size-4" />}
          onClick={() => onDialog({ type: "hide", conversation })}
        >
          {t("menu.hide")}
        </Button>
        {group && (
          <Button
            variant="ghost"
            className="justify-start text-danger"
            icon={<LogOut className="size-4" />}
            onClick={() => onDialog({ type: "leave", conversation })}
          >
            {t("menu.leave")}
          </Button>
        )}
      </section>
    </div>
  );
}

function Shortcut({ icon, label, onClick }: { icon: ReactNode; label: string; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="pressable flex flex-col items-center gap-1 rounded-2xl bg-surface-2 py-3 text-xs font-bold [&>svg]:size-5"
    >
      {icon}
      {label}
    </button>
  );
}

/** A group's icon and name; owner and admins can change both. */
function GroupIdentity() {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me } = useConversationContext();
  const canEdit = manages(conversation.viewer?.role);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(conversation.name);
  const [saving, setSaving] = useState(false);

  const update = async (change: { name?: string; iconAssetId?: string }) => {
    setSaving(true);
    try {
      const response = await conversationClient().updateGroup({
        conversationId: conversation.id,
        ...change,
      });
      if (response.conversation) putConversation(queryClient, response.conversation);
      setRenaming(false);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col items-center gap-2 text-center">
      {canEdit ? (
        <GroupIconPicker
          imageUrl={conversation.icon?.url}
          disabled={saving}
          onUploaded={(assetId) => void update({ iconAssetId: assetId })}
        />
      ) : (
        <ConversationAvatar conversation={conversation} myId={me.id} size="xl" />
      )}
      {renaming ? (
        <form
          className="flex w-full items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (name.trim()) void update({ name: name.trim() });
          }}
        >
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={50}
            aria-label={t("group.name")}
            // biome-ignore lint/a11y/noAutofocus: the field appears because the person asked to rename.
            autoFocus
            className="h-11 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-3 text-center font-bold outline-none focus:border-brand"
          />
          <Button
            type="submit"
            size="icon-sm"
            aria-label={t("actions.save")}
            loading={saving}
            disabled={!name.trim()}
          >
            {!saving && <Check className="size-4" />}
          </Button>
        </form>
      ) : (
        <p className="flex max-w-full items-center gap-1.5 text-xl font-black">
          <span className="truncate">{titleOf(conversation, me.id)}</span>
          {canEdit && (
            <button
              type="button"
              aria-label={t("group.rename")}
              onClick={() => {
                setName(conversation.name);
                setRenaming(true);
              }}
              className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
            >
              <Pencil className="size-4" />
            </button>
          )}
        </p>
      )}
      <p className="-mt-1 text-sm font-bold text-ink-soft">
        {t("group.memberCount", { count: conversation.memberCount })}
      </p>
      {canEdit && conversation.icon?.url && (
        <button
          type="button"
          disabled={saving}
          onClick={() => void update({ iconAssetId: "" })}
          className="text-xs font-bold text-ink-soft underline"
        >
          {t("group.removeIcon")}
        </button>
      )}
    </div>
  );
}

type MemberAction = { type: "remove" | "handOver"; member: chat.Member } | null;

/** The members of a group with their roles, and what the viewer's own role lets them do. */
function Members() {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me, capabilities } = useConversationContext();
  const members = useMembers(conversation.id);
  const [adding, setAdding] = useState(false);
  const [pending, setPending] = useState<MemberAction>(null);
  const [busy, setBusy] = useState(false);
  const rows = members.data?.pages.flatMap((page) => page.members) ?? [];
  const myRole = conversation.viewer?.role;
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: chatKeys.members(conversation.id) });

  const run = async (work: () => Promise<void>, done: string) => {
    setBusy(true);
    try {
      await work();
      await refresh();
      toast.success(done);
      setPending(null);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };
  const setRole = (member: chat.Member, role: chat.MemberRole, done: string) =>
    run(async () => {
      await conversationClient().setMemberRole({
        conversationId: conversation.id,
        userId: member.user!.id,
        role,
      });
    }, done);
  const remove = (member: chat.Member) =>
    run(async () => {
      const response = await conversationClient().removeMember({
        conversationId: conversation.id,
        userId: member.user!.id,
      });
      if (response.conversation) putConversation(queryClient, response.conversation);
    }, t("toast.memberRemoved"));

  const menuFor = (member: chat.Member): MenuAction[] => {
    if (!member.user || member.user.id === me.id) return [];
    const entries: MenuAction[] = [];
    if (myRole === chat.MemberRole.OWNER) {
      entries.push(
        member.role === chat.MemberRole.ADMIN
          ? {
              label: t("members.removeAdmin"),
              icon: <ShieldOff />,
              onSelect: () => void setRole(member, chat.MemberRole.MEMBER, t("toast.roleChanged")),
            }
          : {
              label: t("members.makeAdmin"),
              icon: <Shield />,
              onSelect: () => void setRole(member, chat.MemberRole.ADMIN, t("toast.roleChanged")),
            },
        {
          label: t("members.handOver"),
          icon: <Crown />,
          onSelect: () => setPending({ type: "handOver", member }),
        },
      );
    }
    if (
      myRole === chat.MemberRole.OWNER ||
      (myRole === chat.MemberRole.ADMIN && member.role === chat.MemberRole.MEMBER)
    ) {
      entries.push({
        label: t("members.remove"),
        icon: <UserMinus />,
        danger: true,
        onSelect: () => setPending({ type: "remove", member }),
      });
    }
    return entries;
  };

  return (
    <section>
      <div className="flex items-center">
        <SectionTitle className="flex-1 px-0">
          {t("members.title", { count: conversation.memberCount })}
        </SectionTitle>
        {conversation.memberCount < capabilities.maxGroupMembers && (
          <Button
            size="sm"
            variant="ghost"
            icon={<UserPlus className="size-4" />}
            onClick={() => setAdding(true)}
          >
            {t("members.add")}
          </Button>
        )}
      </div>
      {members.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-12" />
          ))}
        </div>
      ) : (
        <ul className="-mx-2 flex flex-col" data-testid="member-list">
          {rows.map((member) => {
            if (!member.user) return null;
            const menu = menuFor(member);
            return (
              <li
                key={member.user.id}
                className="flex items-center gap-3 rounded-2xl px-2 py-1.5 hover:bg-surface-2"
                data-testid="member-row"
              >
                <Link
                  to="/u/$handle"
                  params={{ handle: member.user.handle }}
                  className="flex min-w-0 flex-1 items-center gap-3"
                >
                  <Avatar user={member.user} size="sm" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-bold">
                      {displayName(member.user)}
                      {member.user.id === me.id && (
                        <span className="text-ink-faint"> ({t("people.you")})</span>
                      )}
                    </span>
                    <span className="block truncate text-xs text-ink-soft">
                      @{member.user.handle}
                    </span>
                  </span>
                </Link>
                {member.role === chat.MemberRole.OWNER && (
                  <Chip className="bg-sun text-[#17122b]">{t("members.owner")}</Chip>
                )}
                {member.role === chat.MemberRole.ADMIN && (
                  <Chip className="bg-sky text-[#17122b]">{t("members.admin")}</Chip>
                )}
                {menu.length > 0 && (
                  <Menu
                    actions={menu}
                    trigger={
                      <button
                        type="button"
                        aria-label={t("members.manage", { name: displayName(member.user) })}
                        className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3"
                      >
                        <MoreHorizontal className="size-4" />
                      </button>
                    }
                  />
                )}
              </li>
            );
          })}
        </ul>
      )}
      <LoadMore query={members} />

      <AddMembersDialog
        open={adding}
        onOpenChange={setAdding}
        memberIds={rows.flatMap((member) => (member.user ? [member.user.id] : []))}
        onAdded={refresh}
      />
      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(open) => !open && setPending(null)}
        title={
          pending?.type === "handOver"
            ? t("members.handOverTitle", { name: displayName(pending.member.user) })
            : t("members.removeTitle", { name: displayName(pending?.member.user) })
        }
        description={
          pending?.type === "handOver" ? t("members.handOverLead") : t("members.removeLead")
        }
        confirmLabel={pending?.type === "handOver" ? t("members.handOver") : t("members.remove")}
        danger={pending?.type === "remove"}
        loading={busy}
        onConfirm={() => {
          if (!pending) return;
          if (pending.type === "handOver")
            void setRole(pending.member, chat.MemberRole.OWNER, t("toast.handedOver"));
          else void remove(pending.member);
        }}
      />
    </section>
  );
}

function AddMembersDialog({
  open,
  onOpenChange,
  memberIds,
  onAdded,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  memberIds: string[];
  onAdded(): void;
}) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, capabilities } = useConversationContext();
  const [chosen, setChosen] = useState<common.UserSummary[]>([]);
  const [adding, setAdding] = useState(false);

  const add = async () => {
    setAdding(true);
    try {
      const response = await conversationClient().addMembers({
        conversationId: conversation.id,
        userIds: chosen.map((user) => user.id),
      });
      if (response.conversation) putConversation(queryClient, response.conversation);
      toast.success(t("toast.membersAdded", { count: response.added.length }));
      setChosen([]);
      onAdded();
      onOpenChange(false);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setAdding(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("members.addTitle")}
      description={t("members.addLead")}
      className="sm:h-[min(40rem,calc(100dvh-4rem))]"
      footer={
        <Button onClick={add} loading={adding} disabled={chosen.length === 0}>
          {t("members.addConfirm", { count: chosen.length })}
        </Button>
      }
    >
      <FriendPicker
        selected={chosen}
        onChange={setChosen}
        excludeIds={memberIds}
        limit={capabilities.maxGroupMembers - conversation.memberCount}
      />
    </Dialog>
  );
}
