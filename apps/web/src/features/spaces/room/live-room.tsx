import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, spaces } from "@tokuly/api";
import { isModerator, nameOf, onStage, patchSpace, rememberSpace } from "@tokuly/core/spaces";
import {
  Ban,
  ChevronDown,
  ExternalLink,
  Hand,
  Headphones,
  LogOut,
  Mic,
  MicOff,
  MoreHorizontal,
  Pencil,
  Pin,
  Power,
  Settings2,
  UserPlus,
  Volume2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { useSubjectMenu } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { useHideTabBar } from "#/stores/shell";
import { spaceClient, useParticipants, useSpacesConfig } from "../api";
import { DevicesDialog } from "../devices-dialog";
import { AudienceLabel, LiveBadge, PersonName, TagList } from "../parts";
import {
  enterSpace,
  expectEnd,
  leaveVenue,
  roleChanged,
  toggleMicrophone,
  unblockAudio,
  useAudioSession,
  useVoices,
} from "../session";
import { SpaceForm } from "../space-form";
import { useRoomAction } from "./actions";
import { AudienceTile, PersonSheet, StageTile } from "./people";
import { ReactionBar, ReactionLayer } from "./reactions";
import { BansDialog, InvitesDialog, PinDialog } from "./room-dialogs";

const Role = spaces.SpaceRole;

type Sheet = "pin" | "invites" | "bans" | "devices" | "edit" | "end" | "leave" | null;

/**
 * A room that is on: the stage, the audience, and what the viewer can do in
 * it. From outside it is a preview with a way in; inside it is the room
 * itself, which keeps playing when the person goes elsewhere in the app.
 */
export function LiveRoom({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const config = useSpacesConfig().data;
  const active = useAudioSession((state) =>
    state.active?.kind === "space" && state.active.id === space.id ? state.active : null,
  );
  const inside = active !== null;
  useHideTabBar();

  const spaceId = space.id;
  const viewer = space.viewer;
  const myRole = viewer?.role ?? Role.UNSPECIFIED;
  const iAmHost = myRole === Role.HOST;
  const moderator = isModerator(myRole);
  const staged = inside && onStage(myRole);
  const voices = useVoices("space", spaceId);
  const everyone = useParticipants(
    spaceId,
    spaces.ParticipantFilter.UNSPECIFIED,
    inside || moderator,
  );
  const listeners = (everyone.data ?? []).filter((entry) => !onStage(entry.role));
  const hands = listeners.filter((entry) => entry.handRaisedAt);
  const quiet = listeners.filter((entry) => !entry.handRaisedAt);

  const [selected, setSelected] = useState<spaces.SpaceParticipant | null>(null);
  const [sheet, setSheet] = useState<Sheet>(null);
  const [entering, setEntering] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const act = useRoomAction(spaceId);
  const subject = { type: common.SubjectType.SPACE, id: spaceId };
  const more = useSubjectMenu(subject, {
    title: t("share.title", { title: space.title }),
    label: space.title,
    report: !iAmHost,
  });

  const enter = async () => {
    setEntering(true);
    await enterSpace(queryClient, space);
    setEntering(false);
  };
  const leave = async () => {
    setLeaving(true);
    await leaveVenue(queryClient);
    void navigate({ to: "/spaces" });
  };
  const end = async () => {
    setLeaving(true);
    try {
      expectEnd(spaceId);
      const response = await spaceClient().endSpace({ spaceId });
      await leaveVenue(queryClient);
      rememberSpace(queryClient, response.space);
      toast.success(t("toast.ended"));
    } catch {
      toast.error(t("toast.endFailed"));
    }
    setLeaving(false);
    setSheet(null);
  };
  const setHand = (raised: boolean) =>
    act.mutate({
      call: () => spaceClient().setHandRaised({ spaceId, raised }),
      after: () =>
        patchSpace(queryClient, spaceId, (loaded) =>
          loaded.viewer ? { ...loaded, viewer: { ...loaded.viewer, handRaised: raised } } : loaded,
        ),
    });
  const answerInvite = (accept: boolean) =>
    act.mutate({
      call: async () => {
        const { me } = await spaceClient().respondToStageInvite({ spaceId, accept });
        patchSpace(queryClient, spaceId, (loaded) =>
          loaded.viewer
            ? {
                ...loaded,
                viewer: {
                  ...loaded.viewer,
                  stageInvited: false,
                  role: me?.role ?? loaded.viewer.role,
                  handRaised: false,
                },
              }
            : loaded,
        );
        if (me && onStage(me.role)) roleChanged(queryClient, spaceId, me.role);
      },
      done: accept ? t("toast.youAreOnStage") : undefined,
    });
  const stepDown = () =>
    act.mutate({
      call: async () => {
        await spaceClient().moveToAudience({ spaceId, userId: "" });
        patchSpace(queryClient, spaceId, (loaded) =>
          loaded.viewer ? { ...loaded, viewer: { ...loaded.viewer, role: Role.LISTENER } } : loaded,
        );
        roleChanged(queryClient, spaceId, Role.LISTENER);
      },
    });

  // In a public room anyone inside may bring friends; otherwise that is the moderators' call.
  const mayInvite = moderator || (inside && space.audience === spaces.SpaceAudience.PUBLIC);
  const menu: MenuAction[] = [
    ...(mayInvite
      ? [{ label: t("menu.invite"), icon: <UserPlus />, onSelect: () => setSheet("invites") }]
      : []),
    ...(moderator
      ? [{ label: t("menu.pin"), icon: <Pin />, onSelect: () => setSheet("pin") }]
      : []),
    ...(iAmHost
      ? [{ label: t("menu.edit"), icon: <Pencil />, onSelect: () => setSheet("edit") }]
      : []),
    ...(moderator
      ? [{ label: t("menu.bans"), icon: <Ban />, onSelect: () => setSheet("bans") }]
      : []),
    ...(inside
      ? [{ label: t("menu.devices"), icon: <Settings2 />, onSelect: () => setSheet("devices") }]
      : []),
    ...(staged && !iAmHost
      ? [{ label: t("menu.stepDown"), icon: <ChevronDown />, onSelect: stepDown }]
      : []),
    ...more.actions,
    ...(iAmHost
      ? [
          {
            label: t("menu.end"),
            icon: <Power />,
            danger: true,
            onSelect: () => setSheet("end"),
          },
        ]
      : []),
  ];

  const hiddenListeners = everyone.hiddenCount;
  return (
    <div
      className="relative flex h-full flex-col"
      data-testid="live-room"
      data-mic-published={active ? String(active.micPublished) : undefined}
    >
      <header className="glass z-20 border-b-2 border-line">
        <div className="mx-auto flex h-16 max-w-5xl items-center gap-2 px-4">
          <Link
            to="/spaces"
            aria-label={t(inside ? "room.minimize" : "room.back")}
            className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
          >
            <ChevronDown className="size-6" />
          </Link>
          <LiveBadge />
          <span
            className="inline-flex items-center gap-1 text-sm font-black text-ink-soft"
            data-testid="listener-count"
          >
            <Headphones className="size-4" aria-hidden />
            <span aria-hidden>{space.listenerCount.toLocaleString()}</span>
            <span className="sr-only">
              {t("room.listenerCount", { count: space.listenerCount })}
            </span>
          </span>
          <span className="flex-1" />
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("room.more")}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={menu}
          />
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid max-w-5xl gap-6 px-4 pt-5 pb-8 lg:grid-cols-[minmax(0,1fr)_20rem]">
          <div className="flex min-w-0 flex-col gap-4">
            <div>
              <AudienceLabel audience={space.audience} className="text-ink-faint" />
              <h1 className="mt-1 text-2xl leading-snug font-black break-words">
                <EmojiText text={space.title} />
              </h1>
              <TagList tags={space.tags} className="mt-2" />
            </div>

            {active?.phase === "reconnecting" && (
              <p
                role="status"
                className="flex items-center gap-2 rounded-2xl bg-sun px-4 py-3 text-sm font-bold text-[#17122b]"
                data-testid="room-reconnecting"
              >
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
                className="pressable sticker flex items-center gap-3 rounded-2xl bg-lime px-4 py-3 text-left text-sm font-black text-[#17122b]"
              >
                <Volume2 className="size-5 shrink-0" />
                {t("room.audioBlocked")}
              </button>
            )}
            {inside && viewer?.stageInvited && !onStage(myRole) && (
              <div
                className="sticker flex animate-pop-in flex-wrap items-center gap-3 rounded-card bg-brand-soft p-4"
                data-testid="stage-invite"
              >
                <span className="text-3xl">🎤</span>
                <p className="min-w-40 flex-1 font-black">{t("room.stageInvite")}</p>
                <Button size="sm" variant="ghost" onClick={() => answerInvite(false)}>
                  {t("room.stageDecline")}
                </Button>
                <Button size="sm" variant="lime" onClick={() => answerInvite(true)}>
                  {t("room.stageAccept")}
                </Button>
              </div>
            )}

            {space.pin && (space.pin.text || space.pin.url) && (
              <PinnedNote pin={space.pin} onEdit={moderator ? () => setSheet("pin") : undefined} />
            )}

            <section aria-label={t("room.stage")}>
              <h2 className="mb-2 flex items-center gap-2 text-sm font-black text-ink-soft">
                <Mic className="size-4" />
                {t("room.stage")}
                <span className="text-ink-faint">
                  {space.stage.length}/{space.maxSpeakers || config?.maxSpeakers || 10}
                </span>
              </h2>
              <ul
                className="grid grid-cols-3 gap-1 sm:grid-cols-4 xl:grid-cols-5"
                data-testid="stage"
              >
                {space.stage.map((entry) => (
                  <StageTile
                    key={entry.user?.id}
                    participant={entry}
                    voices={voices}
                    onOpen={() => setSelected(entry)}
                  />
                ))}
              </ul>
            </section>
          </div>

          <div className="flex min-w-0 flex-col gap-5">
            {moderator && hands.length > 0 && (
              <section aria-label={t("room.hands")} data-testid="hands">
                <h2 className="mb-2 text-sm font-black text-ink-soft">
                  ✋ {t("room.hands")}
                  <span className="ml-2 text-ink-faint">{hands.length}</span>
                </h2>
                <ul className="flex flex-col gap-1">
                  {hands.map((entry) => (
                    <li
                      key={entry.user?.id}
                      className="flex items-center gap-3 rounded-2xl bg-surface px-3 py-2"
                    >
                      <Avatar user={entry.user} size="sm" outlined />
                      <span className="min-w-0 flex-1 truncate text-sm font-bold">
                        <PersonName user={entry.user} />
                      </span>
                      <Button
                        size="sm"
                        variant="lime"
                        disabled={act.isPending}
                        onClick={() =>
                          act.mutate({
                            call: () =>
                              spaceClient().inviteToStage({
                                spaceId,
                                userId: entry.user?.id ?? "",
                              }),
                            done: t("toast.onStage", { name: nameOf(entry.user) }),
                          })
                        }
                      >
                        {t("room.bringUp")}
                      </Button>
                    </li>
                  ))}
                </ul>
              </section>
            )}

            <section aria-label={t("room.audience")}>
              <h2 className="mb-2 flex items-center gap-2 text-sm font-black text-ink-soft">
                <Headphones className="size-4" />
                {t("room.audience")}
              </h2>
              {inside || moderator ? (
                <>
                  {listeners.length > 0 && (
                    <ul
                      className="grid grid-cols-4 gap-1 sm:grid-cols-6 lg:grid-cols-4"
                      data-testid="audience"
                    >
                      {(moderator ? quiet : listeners).map((entry) => (
                        <AudienceTile
                          key={entry.user?.id}
                          participant={entry}
                          voices={voices}
                          onOpen={() => setSelected(entry)}
                        />
                      ))}
                    </ul>
                  )}
                  <LoadMore query={everyone.paging} />
                  {/* People who keep their online status to themselves are counted, never named. */}
                  {hiddenListeners > 0 && (
                    <p className="mt-2 text-xs font-bold text-ink-faint" data-testid="hidden-count">
                      {t(listeners.length > 0 ? "room.hiddenMore" : "room.hiddenOnly", {
                        count: hiddenListeners,
                      })}
                    </p>
                  )}
                  {everyone.data && listeners.length === 0 && hiddenListeners === 0 && (
                    <p className="rounded-2xl border-2 border-dashed border-line px-4 py-5 text-center text-sm font-bold text-ink-faint">
                      {t("room.audienceEmpty")}
                    </p>
                  )}
                </>
              ) : (
                <p className="rounded-2xl bg-surface px-4 py-4 text-sm font-bold text-ink-soft">
                  {t("room.audienceOutside", { count: space.listenerCount })}
                </p>
              )}
            </section>
          </div>
        </div>
      </div>

      <ReactionLayer spaceId={spaceId} />

      <footer className="glass z-20 border-t-2 border-line">
        <div className="mx-auto flex max-w-5xl flex-col gap-3 px-4 pt-3 pb-safe">
          {inside ? (
            <>
              <ReactionBar spaceId={spaceId} />
              <div className="flex items-center gap-2">
                {staged ? (
                  <button
                    type="button"
                    aria-pressed={active.micOn}
                    aria-label={t(active.micOn ? "room.mute" : "room.unmute")}
                    disabled={active.phase !== "connected" || active.paused}
                    onClick={() => void toggleMicrophone(queryClient)}
                    className={cn(
                      "pressable sticker inline-flex h-12 items-center gap-2 rounded-full px-5 font-black disabled:opacity-50",
                      active.micOn ? "bg-lime text-[#17122b]" : "bg-surface text-ink",
                    )}
                    data-testid="mic-toggle"
                  >
                    {active.micOn ? <Mic className="size-5" /> : <MicOff className="size-5" />}
                    {t(active.micOn ? "room.micOn" : "room.micOff")}
                  </button>
                ) : (
                  <button
                    type="button"
                    aria-pressed={Boolean(viewer?.handRaised)}
                    disabled={act.isPending}
                    onClick={() => setHand(!viewer?.handRaised)}
                    className={cn(
                      "pressable sticker inline-flex h-12 items-center gap-2 rounded-full px-5 font-black disabled:opacity-50",
                      viewer?.handRaised ? "bg-sun text-[#17122b]" : "bg-surface text-ink",
                    )}
                    data-testid="hand-toggle"
                  >
                    <Hand className="size-5" />
                    {t(viewer?.handRaised ? "room.handDown" : "room.handUp")}
                  </button>
                )}
                {active.phase === "connecting" && (
                  <span className="flex items-center gap-2 text-sm font-bold text-ink-soft">
                    <Spinner className="size-4" />
                    {t("room.connecting")}
                  </span>
                )}
                <span className="flex-1" />
                <Button
                  variant="secondary"
                  icon={<LogOut className="size-4" />}
                  loading={leaving}
                  onClick={() => (iAmHost ? setSheet("leave") : void leave())}
                  data-testid="leave-room"
                >
                  {t("room.leave")}
                </Button>
              </div>
            </>
          ) : (
            <>
              <Button
                size="lg"
                block
                icon={<Headphones className="size-5" />}
                loading={entering}
                onClick={() => void enter()}
                data-testid="join-room"
              >
                {t(
                  viewer?.inside ? "room.rejoin" : onStage(myRole) ? "room.joinStage" : "room.join",
                )}
              </Button>
              <p className="text-center text-xs font-bold text-ink-faint">{t("room.joinHint")}</p>
            </>
          )}
        </div>
      </footer>

      <PersonSheet
        space={space}
        participant={selected}
        onClose={() => setSelected(null)}
        run={(action) => act.mutate(action)}
      />
      <PinDialog
        space={space}
        open={sheet === "pin"}
        onOpenChange={(open) => !open && setSheet(null)}
      />
      <InvitesDialog
        space={space}
        moderator={moderator}
        open={sheet === "invites"}
        onOpenChange={(open) => !open && setSheet(null)}
      />
      <BansDialog
        space={space}
        open={sheet === "bans"}
        onOpenChange={(open) => !open && setSheet(null)}
      />
      <DevicesDialog open={sheet === "devices"} onOpenChange={(open) => !open && setSheet(null)} />
      <SpaceForm
        open={sheet === "edit"}
        onOpenChange={(open) => !open && setSheet(null)}
        space={space}
      />
      <ConfirmDialog
        open={sheet === "end"}
        onOpenChange={(open) => !open && setSheet(null)}
        title={t("confirm.endTitle")}
        description={t("confirm.endBody")}
        confirmLabel={t("menu.end")}
        danger
        loading={leaving}
        onConfirm={() => void end()}
      />
      <Dialog
        open={sheet === "leave"}
        onOpenChange={(open) => !open && setSheet(null)}
        title={t("hostLeave.title")}
        description={t("hostLeave.body", {
          minutes: Math.max(1, Math.round((config?.hostGraceSeconds || 180) / 60)),
        })}
      >
        <div className="flex flex-col gap-2">
          <Button variant="danger" block loading={leaving} onClick={() => void end()}>
            {t("hostLeave.end")}
          </Button>
          <Button variant="secondary" block disabled={leaving} onClick={() => void leave()}>
            {t("hostLeave.leave")}
          </Button>
        </div>
      </Dialog>
      {more.dialogs}
    </div>
  );
}

function PinnedNote({ pin, onEdit }: { pin: spaces.SpacePin; onEdit?: () => void }) {
  const { t } = useTranslation("spaces");
  return (
    <aside
      className="sticker-sm flex items-start gap-3 rounded-2xl bg-sun/25 p-3.5"
      aria-label={t("pin.label")}
      data-testid="room-pin"
    >
      <Pin className="mt-0.5 size-4 shrink-0 text-ink-soft" />
      <div className="min-w-0 flex-1">
        {pin.text && (
          <p className="text-sm leading-relaxed font-bold break-words whitespace-pre-wrap">
            <EmojiText text={pin.text} />
          </p>
        )}
        {pin.url && (
          <a
            href={pin.url}
            target="_blank"
            rel="noopener noreferrer nofollow"
            className="mt-1 inline-flex max-w-full items-center gap-1 text-sm font-bold text-brand-strong underline"
          >
            <span className="truncate">{pin.url.replace(/^https?:\/\//, "")}</span>
            <ExternalLink className="size-3.5 shrink-0" />
          </a>
        )}
      </div>
      {onEdit && (
        <button
          type="button"
          onClick={onEdit}
          aria-label={t("menu.pin")}
          className="grid size-8 shrink-0 place-items-center rounded-full hover:bg-surface-2"
        >
          <Pencil className="size-4" />
        </button>
      )}
    </aside>
  );
}
