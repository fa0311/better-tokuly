import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, spaces } from "@tokuly/api";
import {
  Bell,
  BellOff,
  Crown,
  DoorOpen,
  LogOut,
  Mic,
  MicOff,
  MoreHorizontal,
  Pencil,
  Settings2,
  Trash2,
  UserMinus,
  UserPlus,
  Volume2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Spinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { useSubjectMenu } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { forgetHangout, hangoutClient, keys, rememberHangout, useHangout } from "../api";
import { DevicesDialog } from "../devices-dialog";
import { ExitNotice } from "../exit-notice";
import { FriendPicker } from "../friend-picker";
import { insideOf, nameOf } from "../model";
import { PersonName } from "../parts";
import {
  acknowledgeExit,
  enterHangout,
  leaveVenue,
  toggleMicrophone,
  unblockAudio,
  useAudioSession,
  useVoices,
} from "../session";
import { HangoutForm } from "./hangout-form";
import { useRespondToInvite } from "./hangouts-tab";

type Sheet = "invite" | "edit" | "devices" | "quit" | "delete" | null;
type Confirm = { kind: "remove" | "transfer"; member: spaces.HangoutMember } | null;

const State = spaces.HangoutMemberState;

/** A friend group's always-on voice room: who is inside right now, and the door. */
export function HangoutScreen({ hangoutId }: { hangoutId: string }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const query = useHangout(hangoutId);
  const hangout = query.data;
  const exit = useAudioSession((state) =>
    state.exit?.kind === "hangout" && state.exit.id === hangoutId ? state.exit : null,
  );

  if (exit) {
    const member = hangout?.viewer?.state === State.MEMBER;
    return (
      <ExitNotice
        reason={exit.reason}
        action={
          member &&
          exit.reason !== "closed" && (
            <Button
              onClick={() => {
                acknowledgeExit();
                void enterHangout(queryClient, hangout);
              }}
            >
              {t("exit.reconnect")}
            </Button>
          )
        }
      />
    );
  }
  if (query.isPending)
    return (
      <Page title={t("tabs.hangouts")} back="/spaces">
        <Skeleton className="h-64" />
      </Page>
    );
  if (!hangout)
    return (
      <Page title={t("tabs.hangouts")} back="/spaces">
        <EmptyState emoji="🫥" title={t("hangout.notFound")} body={t("hangout.notFoundBody")} />
      </Page>
    );
  return <Loaded hangout={hangout} />;
}

function Loaded({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const myId = useMe()?.user?.id ?? "";
  const hangoutId = hangout.id;
  const viewer = hangout.viewer;
  const owner = Boolean(viewer?.owner);
  const invited = viewer?.state === State.INVITED;
  const active = useAudioSession((state) =>
    state.active?.kind === "hangout" && state.active.id === hangoutId ? state.active : null,
  );
  const voices = useVoices("hangout", hangoutId);
  const respond = useRespondToInvite(hangoutId);
  const [sheet, setSheet] = useState<Sheet>(null);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [entering, setEntering] = useState(false);

  const members = hangout.members.filter((member) => member.state === State.MEMBER);
  const pending = hangout.members.filter((member) => member.state === State.INVITED);
  const inside = insideOf(hangout);
  const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "spaces" }));
  const toList = () => void navigate({ to: "/spaces", search: { tab: "hangouts" } });

  /** A change that answers with the hangout as it is afterwards. */
  const change = useMutation({
    mutationFn: (run: { call(): Promise<{ hangout?: spaces.Hangout }>; done?: string }) =>
      run.call(),
    onSuccess: (response, run) => {
      rememberHangout(queryClient, response.hangout);
      setSheet(null);
      setConfirm(null);
      if (run.done) toast.success(run.done);
    },
    onError: fail,
  });
  /** Leaving the group for good, one way or the other. */
  const part = useMutation({
    mutationFn: async (kind: "quit" | "delete") => {
      if (active) await leaveVenue(queryClient);
      if (kind === "quit") await hangoutClient().quitHangout({ hangoutId });
      else await hangoutClient().deleteHangout({ hangoutId });
    },
    onSuccess: (_result, kind) => {
      forgetHangout(queryClient, hangoutId);
      void queryClient.invalidateQueries({ queryKey: keys.hangouts });
      toast.success(t(kind === "quit" ? "toast.hangoutQuit" : "toast.hangoutDeleted"));
      toList();
    },
    onError: fail,
  });

  const subject = { type: common.SubjectType.SPACE, id: hangoutId };
  const more = useSubjectMenu(subject, { label: hangout.name, share: false, report: !owner });
  const menu: MenuAction[] = invited
    ? more.actions
    : [
        ...(viewer?.canInvite
          ? [{ label: t("hangout.invite"), icon: <UserPlus />, onSelect: () => setSheet("invite") }]
          : []),
        ...(owner
          ? [{ label: t("hangout.edit"), icon: <Pencil />, onSelect: () => setSheet("edit") }]
          : []),
        {
          label: t(viewer?.muted ? "hangout.unmuteNotices" : "hangout.muteNotices"),
          icon: viewer?.muted ? <Bell /> : <BellOff />,
          onSelect: () =>
            change.mutate({
              call: () => hangoutClient().setHangoutMuted({ hangoutId, muted: !viewer?.muted }),
              done: t(viewer?.muted ? "toast.noticesOn" : "toast.noticesOff"),
            }),
        },
        ...(active
          ? [
              {
                label: t("menu.devices"),
                icon: <Settings2 />,
                onSelect: () => setSheet("devices"),
              },
            ]
          : []),
        ...more.actions,
        owner
          ? {
              label: t("hangout.delete"),
              icon: <Trash2 />,
              danger: true,
              onSelect: () => setSheet("delete"),
            }
          : {
              label: t("hangout.quit"),
              icon: <LogOut />,
              danger: true,
              onSelect: () => setSheet("quit"),
            },
      ];

  return (
    <Page
      title={
        <span className="flex items-center gap-2">
          <span>{hangout.emoji || "🛋️"}</span>
          <span className="truncate">
            <EmojiText text={hangout.name} />
          </span>
        </span>
      }
      back="/spaces"
      actions={
        <Menu
          trigger={
            <Button variant="ghost" size="icon-sm" aria-label={t("room.more")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={menu}
        />
      }
    >
      {invited ? (
        <Card className="flex flex-col items-center gap-3 text-center" data-testid="hangout-invite">
          <div className="text-5xl">{hangout.emoji || "🛋️"}</div>
          <p className="text-lg font-black">
            {t("hangout.invitedTitle", { name: nameOf(hangout.owner) })}
          </p>
          <p className="text-sm text-ink-soft">{t("hangout.invitedBody")}</p>
          <div className="flex gap-2">
            <Button
              variant="lime"
              loading={respond.isPending && respond.variables === true}
              disabled={respond.isPending}
              onClick={() => respond.mutate(true)}
            >
              {t("hangouts.accept")}
            </Button>
            <Button
              variant="ghost"
              disabled={respond.isPending}
              onClick={() => respond.mutate(false, { onSuccess: toList })}
            >
              {t("hangouts.decline")}
            </Button>
          </div>
        </Card>
      ) : (
        <Card
          className={cn("flex flex-col gap-4", inside.length > 0 && "bg-lime text-[#17122b]")}
          data-testid="hangout-room"
        >
          <div>
            <p className="text-sm font-black opacity-70">
              {t(inside.length > 0 ? "hangout.open" : "hangout.closed")}
            </p>
            <p className="text-xl font-black" data-testid="hangout-inside-count">
              {inside.length > 0
                ? t("hangouts.inside", { count: hangout.insideCount })
                : t("hangout.nobody")}
            </p>
          </div>
          {inside.length > 0 && (
            <ul className="flex flex-wrap gap-3" data-testid="hangout-inside">
              {inside.map((member) => {
                const id = member.user?.id ?? "";
                const speaking = voices.speaking.has(id);
                const muted = voices.connected.has(id) && !voices.open.has(id);
                return (
                  <li
                    key={id}
                    className="flex w-16 flex-col items-center gap-1"
                    data-speaking={speaking || undefined}
                    data-mic={voices.open.has(id) ? "on" : undefined}
                  >
                    <span className="relative">
                      <Avatar
                        user={member.user}
                        size="lg"
                        outlined
                        className={cn("rounded-[1.25rem]", speaking && "ring-4 ring-brand")}
                      />
                      {muted && (
                        <span
                          className="absolute -right-1.5 -bottom-1.5 grid size-6 place-items-center rounded-full border-2 border-ink bg-surface text-ink-soft"
                          role="img"
                          aria-label={t("room.mutedLabel")}
                        >
                          <MicOff className="size-3.5" />
                        </span>
                      )}
                    </span>
                    <span className="w-full truncate text-center text-xs font-bold">
                      <PersonName user={member.user} />
                    </span>
                  </li>
                );
              })}
            </ul>
          )}
          {active?.phase === "reconnecting" && (
            <p role="status" className="flex items-center gap-2 text-sm font-bold">
              <Spinner className="size-4" />
              {t("room.reconnecting")}
            </p>
          )}
          {active?.paused && (
            <p
              role="status"
              className="flex items-center gap-2 rounded-2xl bg-sky px-4 py-3 text-sm font-bold text-[#17122b]"
              data-testid="room-paused"
            >
              <span aria-hidden>📞</span>
              {t("room.pausedForCall")}
            </p>
          )}
          {active?.audioBlocked && (
            <button
              type="button"
              onClick={() => void unblockAudio()}
              className="pressable sticker-sm flex items-center gap-2 rounded-2xl bg-surface px-4 py-3 text-left text-sm font-black text-ink"
            >
              <Volume2 className="size-5 shrink-0" />
              {t("room.audioBlocked")}
            </button>
          )}
          <div className="flex flex-wrap items-center gap-2">
            {active ? (
              <>
                <button
                  type="button"
                  aria-pressed={active.micOn}
                  aria-label={t(active.micOn ? "room.mute" : "room.unmute")}
                  disabled={active.phase !== "connected" || active.paused}
                  onClick={() => void toggleMicrophone(queryClient)}
                  className={cn(
                    "pressable sticker inline-flex h-12 items-center gap-2 rounded-full px-5 font-black disabled:opacity-50",
                    active.micOn ? "bg-brand text-on-brand" : "bg-surface text-ink",
                  )}
                  data-testid="mic-toggle"
                >
                  {active.micOn ? <Mic className="size-5" /> : <MicOff className="size-5" />}
                  {t(active.micOn ? "room.micOn" : "room.micOff")}
                </button>
                <Button
                  variant="secondary"
                  icon={<LogOut className="size-4" />}
                  onClick={() => void leaveVenue(queryClient)}
                  data-testid="leave-hangout"
                >
                  {t("hangout.leave")}
                </Button>
              </>
            ) : (
              <Button
                size="lg"
                icon={<DoorOpen className="size-5" />}
                loading={entering}
                onClick={async () => {
                  setEntering(true);
                  await enterHangout(queryClient, hangout);
                  setEntering(false);
                }}
                data-testid="join-hangout"
              >
                {t(inside.length > 0 ? "hangout.join" : "hangout.joinFirst")}
              </Button>
            )}
          </div>
        </Card>
      )}

      <section className="mt-6">
        <h2 className="text-lg font-black">
          {t("hangout.members")}
          <span className="ml-2 text-sm text-ink-faint">{hangout.memberCount}</span>
        </h2>
        <ul className="mt-2 flex flex-col" data-testid="hangout-members">
          {members.map((member) => (
            <MemberRow
              key={member.user?.id}
              member={member}
              note={
                member.owner
                  ? t("hangout.owner")
                  : member.inside
                    ? t("hangout.insideNow")
                    : undefined
              }
              actions={
                owner && member.user?.id !== myId
                  ? [
                      {
                        label: t("hangout.transfer"),
                        icon: <Crown />,
                        onSelect: () => setConfirm({ kind: "transfer", member }),
                      },
                      {
                        label: t("hangout.remove"),
                        icon: <UserMinus />,
                        danger: true,
                        onSelect: () => setConfirm({ kind: "remove", member }),
                      },
                    ]
                  : []
              }
            />
          ))}
        </ul>
        {pending.length > 0 && (
          <>
            <h3 className="mt-4 text-sm font-black text-ink-soft">{t("hangout.pending")}</h3>
            <ul className="mt-1 flex flex-col">
              {pending.map((member) => (
                <MemberRow
                  key={member.user?.id}
                  member={member}
                  note={t("hangout.waiting")}
                  actions={
                    // The owner takes any invitation back; a member the ones they sent.
                    owner || member.invitedById === myId
                      ? [
                          {
                            label: t("hangout.withdraw"),
                            icon: <UserMinus />,
                            danger: true,
                            onSelect: () =>
                              change.mutate({
                                call: () =>
                                  hangoutClient().removeHangoutMember({
                                    hangoutId,
                                    userId: member.user?.id ?? "",
                                  }),
                              }),
                          },
                        ]
                      : []
                  }
                />
              ))}
            </ul>
          </>
        )}
      </section>

      <FriendPicker
        open={sheet === "invite"}
        onOpenChange={(open) => !open && setSheet(null)}
        title={t("hangout.invite")}
        description={t("hangout.inviteBody")}
        exclude={hangout.members.flatMap((member) => (member.user ? [member.user.id] : []))}
        confirmLabel={t("invites.send")}
        loading={change.isPending}
        onConfirm={(users) => {
          if (users.length === 0) return setSheet(null);
          change.mutate({
            call: () =>
              hangoutClient().inviteToHangout({
                hangoutId,
                userIds: users.map((user) => user.id),
              }),
            done: t("toast.invited", { count: users.length }),
          });
        }}
      />
      <HangoutForm
        open={sheet === "edit"}
        onOpenChange={(open) => !open && setSheet(null)}
        hangout={hangout}
      />
      <DevicesDialog open={sheet === "devices"} onOpenChange={(open) => !open && setSheet(null)} />
      <ConfirmDialog
        open={sheet === "quit" || sheet === "delete"}
        onOpenChange={(open) => !open && setSheet(null)}
        title={t(sheet === "delete" ? "confirm.hangoutDeleteTitle" : "confirm.hangoutQuitTitle")}
        description={t(
          sheet === "delete" ? "confirm.hangoutDeleteBody" : "confirm.hangoutQuitBody",
        )}
        confirmLabel={t(sheet === "delete" ? "hangout.delete" : "hangout.quit")}
        danger
        loading={part.isPending}
        onConfirm={() => part.mutate(sheet === "delete" ? "delete" : "quit")}
      />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => !open && setConfirm(null)}
        title={t(
          confirm?.kind === "transfer"
            ? "confirm.hangoutTransferTitle"
            : "confirm.hangoutRemoveTitle",
          { name: nameOf(confirm?.member.user) },
        )}
        description={t(
          confirm?.kind === "transfer"
            ? "confirm.hangoutTransferBody"
            : "confirm.hangoutRemoveBody",
        )}
        confirmLabel={t(confirm?.kind === "transfer" ? "hangout.transfer" : "hangout.remove")}
        danger={confirm?.kind === "remove"}
        loading={change.isPending}
        onConfirm={() => {
          if (!confirm) return;
          const userId = confirm.member.user?.id ?? "";
          change.mutate({
            call: () =>
              confirm.kind === "transfer"
                ? hangoutClient().transferHangout({ hangoutId, userId })
                : hangoutClient().removeHangoutMember({ hangoutId, userId }),
          });
        }}
      />
      {more.dialogs}
    </Page>
  );
}

function MemberRow({
  member,
  note,
  actions,
}: {
  member: spaces.HangoutMember;
  note?: string;
  actions: MenuAction[];
}) {
  const { t } = useTranslation("spaces");
  const user = member.user;
  return (
    <li className="flex items-center gap-3 rounded-2xl px-2 py-2">
      <Link
        to="/u/$handle"
        params={{ handle: user?.handle ?? "" }}
        className="flex min-w-0 flex-1 items-center gap-3"
      >
        <Avatar user={user} outlined />
        <span className="min-w-0">
          <span className="block truncate font-bold">
            <PersonName user={user} />
          </span>
          <span className="block truncate text-sm text-ink-soft">{note ?? `@${user?.handle}`}</span>
        </span>
      </Link>
      {actions.length > 0 && (
        <Menu
          trigger={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("hangout.memberActions", { name: nameOf(user) })}
            >
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={actions}
        />
      )}
    </li>
  );
}
