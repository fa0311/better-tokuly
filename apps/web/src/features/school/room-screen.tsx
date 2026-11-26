import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, school } from "@tokuly/api";
import { pushesDuringReadByKey } from "@tokuly/core/query";
import {
  DoorOpen,
  Flag,
  LogOut,
  MoreHorizontal,
  Pause,
  Play,
  RotateCcw,
  Send,
  Settings2,
  SkipForward,
  UserMinus,
  UserPlus,
  X,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { ShareToChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { ReportDialog } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { realtime } from "#/lib/realtime";
import { useMe } from "#/stores/session";
import { keys, refresh, roomClient, toastError } from "./api";
import { studyTime } from "./dates";
import { Block, FriendPicker, SchoolPage, SubjectPill, useNow } from "./parts";
import { audienceKey, type RoomForm, RoomFormFields } from "./rooms-screen";
import { setActiveSession, useActiveSession } from "./session-store";
import { SubjectSelect } from "./subject-select";
import { isBreak, studiedSecondsAt, syncClock, timerAt } from "./timer";
import { TimerFace } from "./timer-card";

const REACTIONS: { reaction: school.RoomReaction; emoji: string; key: string }[] = [
  { reaction: school.RoomReaction.CHEER, emoji: "📣", key: "cheer" },
  { reaction: school.RoomReaction.CLAP, emoji: "👏", key: "clap" },
  { reaction: school.RoomReaction.FIRE, emoji: "🔥", key: "fire" },
  { reaction: school.RoomReaction.COFFEE, emoji: "☕", key: "coffee" },
  { reaction: school.RoomReaction.WAVE, emoji: "👋", key: "wave" },
];

interface Burst {
  id: number;
  emoji: string;
  text: string;
}

let burstId = 0;

/** Timer changes pushed while a room is being read (see `pushesDuringRead`). */
const timerPushesDuringRead = pushesDuringReadByKey<school.StudyRoom>();

/**
 * A study room: the shared timer everyone follows, who is here and what
 * they study, the fixed reactions. The host runs the timer, invites and
 * removes people and closes the room; everyone else joins, says what they
 * study, cheers, and leaves.
 */
export function RoomScreen({ roomId }: { roomId: string }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const me = useMe();
  const now = useNow(250);
  const mySession = useActiveSession((state) => state.session);
  const key = keys.room(roomId);
  const {
    data: room,
    isPending,
    isError,
  } = useQuery({
    queryKey: key,
    queryFn: async () => {
      const read = timerPushesDuringRead.begin(roomId);
      try {
        const response = await roomClient().getStudyRoom({ roomId });
        // A timer change pushed while the room was read is newer than the answer.
        const room = response.room && read.finish(response.room);
        read.cancel();
        syncClock(room?.timer);
        return room;
      } catch (error) {
        read.cancel();
        throw error;
      }
    },
    staleTime: 0,
  });
  const [closed, setClosed] = useState(false);
  const [bursts, setBursts] = useState<Burst[]>([]);
  const [joinSubject, setJoinSubject] = useState("");
  const [dialog, setDialog] = useState<
    "settings" | "invite" | "close" | "report" | "share" | undefined
  >(undefined);
  const [member, setMember] = useState<school.RoomMember | undefined>(undefined);

  // Live: the timer, presence and reactions of this room.
  useEffect(() => {
    const client = realtime();
    const topic = `study:${roomId}`;
    const reread = () => void queryClient.invalidateQueries({ queryKey: keys.room(roomId) });
    const { release } = client.subscribe(topic, {
      // In effect (first, or back after a gap): the room as read before may lack a timer change
      // pushed in between.
      onSubscribed: reread,
      // Removed, uninvited, unfriended or the room closed: reading it again shows where things stand.
      onRevoked: reread,
    });
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "school" || frame.topic !== topic) return;
      const event = frame.body.value.event;
      if (event.case === "roomTimer") {
        syncClock(event.value);
        const timer = event.value;
        const update = (current: school.StudyRoom) => ({ ...current, timer });
        timerPushesDuringRead.record(roomId, update);
        queryClient.setQueryData<school.StudyRoom>(keys.room(roomId), (current) =>
          current ? update(current) : current,
        );
      }
      if (event.case === "roomReaction") {
        const reaction = REACTIONS.find((item) => item.reaction === event.value.reaction);
        if (reaction) {
          const burst = {
            id: ++burstId,
            emoji: reaction.emoji,
            text: `${event.value.from?.displayName ?? ""}: ${t(`rooms.reactions.${reaction.key}`)}`,
          };
          setBursts((current) => [...current.slice(-4), burst]);
          window.setTimeout(
            () => setBursts((current) => current.filter((item) => item.id !== burst.id)),
            3500,
          );
        }
        return;
      }
      if (event.case === "roomClosed") setClosed(true);
      // Presence and study times are the server's to say: ask for the room again.
      void queryClient.invalidateQueries({ queryKey: keys.room(roomId) });
    });
    client.start();
    return () => {
      off();
      release();
    };
  }, [roomId, queryClient, t]);

  const leaveToList = () => {
    refresh(queryClient, "rooms", "study");
    void navigate({ to: "/school/rooms" });
  };
  const join = useMutation({
    mutationFn: () => roomClient().joinStudyRoom({ roomId, subjectId: joinSubject }),
    onSuccess: (response) => {
      setActiveSession(response.session, response.heartbeatIntervalSeconds);
      syncClock(response.room?.timer);
      queryClient.setQueryData(key, response.room);
      refresh(queryClient, "study");
    },
    onError: toastError,
  });
  const leave = useMutation({
    mutationFn: () => roomClient().leaveStudyRoom({}),
    onSuccess: (response) => {
      setActiveSession(undefined);
      toast.success(
        t("rooms.leftToast", {
          time: studyTime(response.session?.studiedSeconds ?? 0, i18n.language),
        }),
      );
      leaveToList();
    },
    onError: toastError,
  });
  const close = useMutation({
    mutationFn: () => roomClient().closeStudyRoom({ roomId }),
    onSuccess: () => {
      setActiveSession(undefined);
      toast.success(t("rooms.closedToast"));
      leaveToList();
    },
    onError: toastError,
  });
  const control = useMutation({
    mutationFn: (action: school.TimerAction) =>
      roomClient().controlStudyRoomTimer({ roomId, action }),
    onSuccess: (response) => {
      syncClock(response.timer);
      queryClient.setQueryData<school.StudyRoom>(key, (current) =>
        current && response.timer ? { ...current, timer: response.timer } : current,
      );
    },
    onError: toastError,
  });
  const react = useMutation({
    mutationFn: (input: { reaction: school.RoomReaction; toUserId?: string }) =>
      roomClient().sendStudyRoomReaction({
        reaction: input.reaction,
        toUserId: input.toUserId ?? "",
      }),
    onError: toastError,
  });
  const setSubject = useMutation({
    mutationFn: (subjectId: string) => roomClient().setStudyRoomSubject({ subjectId }),
    onSuccess: (response) => {
      setActiveSession(response.session);
      void queryClient.invalidateQueries({ queryKey: key });
    },
    onError: toastError,
  });
  const removeMember = useMutation({
    mutationFn: (userId: string) => roomClient().removeStudyRoomMember({ roomId, userId }),
    onSuccess: () => {
      setMember(undefined);
      void queryClient.invalidateQueries({ queryKey: key });
    },
    onError: toastError,
  });

  if (isPending) {
    return (
      <SchoolPage back="/school/rooms" title={t("rooms.one")}>
        <Skeleton className="h-80" />
      </SchoolPage>
    );
  }
  if (isError || !room || closed || room.closedAt) {
    return (
      <SchoolPage back="/school/rooms" title={t("rooms.one")}>
        <EmptyState
          emoji="🚪"
          title={closed || room?.closedAt ? t("rooms.closedTitle") : t("errors.ROOM_NOT_FOUND")}
          body={t("rooms.closedBody")}
          action={<Button onClick={leaveToList}>{t("rooms.backToList")}</Button>}
        />
      </SchoolPage>
    );
  }

  const isHost = room.host?.id === me?.user?.id;
  const view = timerAt(room.timer, now);
  const pomodoro = view.mode === school.StudyMode.POMODORO;
  const working = view.state === school.TimerState.RUNNING && !isBreak(view.phase);
  const sinceSeconds = Math.floor(view.workSinceMs / 1000);
  const audience = audienceKey(room.audience);

  return (
    <SchoolPage
      back="/school/rooms"
      title={room.title}
      wide
      actions={
        <Menu
          trigger={
            <Button size="icon-sm" variant="ghost" aria-label={t("rooms.menu")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            { label: t("rooms.shareToChat"), icon: <Send />, onSelect: () => setDialog("share") },
            ...(isHost
              ? [
                  {
                    label: t("rooms.invite"),
                    icon: <UserPlus />,
                    onSelect: () => setDialog("invite"),
                  },
                  {
                    label: t("rooms.settings"),
                    icon: <Settings2 />,
                    onSelect: () => setDialog("settings"),
                  },
                  {
                    label: t("rooms.close"),
                    icon: <XCircle />,
                    danger: true,
                    onSelect: () => setDialog("close"),
                  },
                ]
              : [
                  {
                    label: t("rooms.report"),
                    icon: <Flag />,
                    danger: true,
                    onSelect: () => setDialog("report"),
                  },
                ]),
          ]}
        />
      }
    >
      <div className="grid gap-6 lg:grid-cols-[minmax(0,24rem)_minmax(0,1fr)] lg:items-start">
        <section
          aria-label={t("rooms.sharedTimer")}
          className="sticker relative flex flex-col gap-4 overflow-hidden rounded-card bg-surface p-5"
        >
          <p className="flex flex-wrap items-center justify-center gap-1.5">
            <Chip>{t(`rooms.audience.${audience}`)}</Chip>
            <Chip>{t("rooms.memberCount", { count: room.memberCount, cap: room.memberCap })}</Chip>
            {!room.hostPresent && <Chip>{t("rooms.hostAway")}</Chip>}
          </p>
          <TimerFace
            view={view}
            studiedSeconds={
              room.joined && mySession?.roomId === room.id
                ? studiedSecondsAt(mySession, now)
                : undefined
            }
          />
          <ul
            aria-live="polite"
            className="pointer-events-none absolute inset-x-0 top-3 flex flex-col items-center gap-1"
          >
            {bursts.map((burst) => (
              <li
                key={burst.id}
                className="sticker-sm animate-pop-in flex items-center gap-1.5 rounded-full bg-sun px-3 py-1 text-sm font-black text-[#17122b]"
              >
                <span className="text-lg">{burst.emoji}</span>
                {burst.text}
              </li>
            ))}
          </ul>

          {isHost && room.joined && (
            <div className="flex flex-wrap items-center justify-center gap-2">
              {view.state === school.TimerState.IDLE && (
                <Button
                  variant="lime"
                  icon={<Play className="size-4" />}
                  loading={control.isPending}
                  onClick={() => control.mutate(school.TimerAction.START)}
                >
                  {t("rooms.timer.start")}
                </Button>
              )}
              {view.state === school.TimerState.RUNNING && (
                <Button
                  variant="secondary"
                  icon={<Pause className="size-4" />}
                  loading={control.isPending}
                  onClick={() => control.mutate(school.TimerAction.PAUSE)}
                >
                  {t("timer.pause")}
                </Button>
              )}
              {view.state === school.TimerState.PAUSED && (
                <Button
                  variant="lime"
                  icon={<Play className="size-4" />}
                  loading={control.isPending}
                  onClick={() => control.mutate(school.TimerAction.RESUME)}
                >
                  {t("timer.resume")}
                </Button>
              )}
              {pomodoro && view.state === school.TimerState.RUNNING && (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("timer.skip")}
                  disabled={control.isPending}
                  onClick={() => control.mutate(school.TimerAction.SKIP)}
                >
                  <SkipForward className="size-5" />
                </Button>
              )}
              {view.state !== school.TimerState.IDLE && (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("rooms.timer.reset")}
                  disabled={control.isPending}
                  onClick={() => control.mutate(school.TimerAction.RESET)}
                >
                  <RotateCcw className="size-5" />
                </Button>
              )}
            </div>
          )}
          {!isHost && room.joined && view.state === school.TimerState.IDLE && (
            <p className="text-center text-sm text-ink-soft">
              {t("rooms.waitingForHost", { name: room.host?.displayName ?? "" })}
            </p>
          )}

          {room.joined ? (
            <>
              <div className="flex justify-center gap-1.5">
                {REACTIONS.map((item) => (
                  <button
                    key={item.key}
                    type="button"
                    aria-label={t(`rooms.reactions.${item.key}`)}
                    title={t(`rooms.reactions.${item.key}`)}
                    onClick={() => react.mutate({ reaction: item.reaction })}
                    className="pressable sticker-sm grid size-11 place-items-center rounded-full bg-surface text-xl"
                  >
                    {item.emoji}
                  </button>
                ))}
              </div>
              <SubjectSelect
                label={t("rooms.mySubject")}
                value={mySession?.subject?.id ?? ""}
                onChange={(subjectId) => setSubject.mutate(subjectId)}
                disabled={setSubject.isPending}
              />
              <Button
                variant="secondary"
                icon={<LogOut className="size-4" />}
                loading={leave.isPending}
                onClick={() => leave.mutate()}
              >
                {t("rooms.leave")}
              </Button>
            </>
          ) : (
            <>
              <SubjectSelect
                label={t("rooms.mySubject")}
                value={joinSubject}
                onChange={setJoinSubject}
              />
              <Button
                size="lg"
                icon={<DoorOpen className="size-5" />}
                loading={join.isPending}
                disabled={room.memberCount >= room.memberCap}
                onClick={() => join.mutate()}
              >
                {room.memberCount >= room.memberCap ? t("rooms.full") : t("rooms.join")}
              </Button>
              {mySession && (
                <p className="text-center text-xs text-ink-faint">{t("rooms.joinEndsSession")}</p>
              )}
            </>
          )}
        </section>

        <Block title={t("rooms.members", { count: room.memberCount })} className="mt-0">
          {room.members.length === 0 ? (
            <p className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft">
              {t("rooms.nobodyHere")}
            </p>
          ) : (
            <ul
              className="grid gap-2 sm:grid-cols-2"
              aria-label={t("rooms.members", { count: room.memberCount })}
            >
              {room.members.map((item) => {
                const self = item.user?.id === me?.user?.id;
                const body = (
                  <>
                    <Avatar user={item.user} outlined />
                    <span className="min-w-0 flex-1 text-left">
                      <span className="flex items-center gap-1.5">
                        <span className="truncate font-bold">
                          <EmojiText text={item.user?.displayName ?? ""} />
                        </span>
                        {item.host && (
                          <span className="shrink-0 rounded-full bg-ink px-1.5 text-[10px] font-black text-surface">
                            {t("rooms.hostBadge")}
                          </span>
                        )}
                      </span>
                      <span className="mt-0.5 flex items-center gap-1.5 text-xs font-bold text-ink-soft">
                        {item.subject && (
                          <SubjectPill
                            subject={item.subject}
                            className="h-5 border px-1.5 text-[11px]"
                          />
                        )}
                        <span>
                          {studyTime(
                            item.studiedSeconds + (working ? sinceSeconds : 0),
                            i18n.language,
                          )}
                        </span>
                      </span>
                    </span>
                  </>
                );
                const className = cn(
                  "flex w-full items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5",
                  self && "sticker-sm bg-surface",
                );
                return (
                  <li key={item.user?.id}>
                    {room.joined && !self ? (
                      <button
                        type="button"
                        onClick={() => setMember(item)}
                        className={cn(className, "transition-colors hover:bg-surface-3")}
                      >
                        {body}
                      </button>
                    ) : (
                      <div className={className}>{body}</div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </Block>
      </div>

      {member && (
        <Dialog
          open
          onOpenChange={(open) => !open && setMember(undefined)}
          title={member.user?.displayName ?? ""}
          description={t("rooms.reactTo")}
        >
          <div className="flex flex-col gap-4">
            <div className="flex flex-wrap justify-center gap-2">
              {REACTIONS.map((item) => (
                <Button
                  key={item.key}
                  variant="secondary"
                  onClick={() => {
                    react.mutate({ reaction: item.reaction, toUserId: member.user?.id });
                    setMember(undefined);
                  }}
                >
                  {item.emoji} {t(`rooms.reactions.${item.key}`)}
                </Button>
              ))}
            </div>
            {isHost && (
              <Button
                variant="ghost"
                className="text-danger"
                icon={<UserMinus className="size-4" />}
                loading={removeMember.isPending}
                onClick={() => member.user && removeMember.mutate(member.user.id)}
              >
                {t("rooms.removeMember")}
              </Button>
            )}
          </div>
        </Dialog>
      )}
      {dialog === "settings" && (
        <RoomSettingsDialog room={room} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "invite" && <InviteDialog room={room} onClose={() => setDialog(undefined)} />}
      <ConfirmDialog
        open={dialog === "close"}
        onOpenChange={(open) => !open && setDialog(undefined)}
        title={t("rooms.closeTitle")}
        description={t("rooms.closeBody")}
        confirmLabel={t("rooms.close")}
        danger
        loading={close.isPending}
        onConfirm={() => close.mutate()}
      />
      <ReportDialog
        subject={{ type: common.SubjectType.STUDY_ROOM, id: room.id }}
        label={room.title}
        open={dialog === "report"}
        onOpenChange={(open) => !open && setDialog(undefined)}
      />
      <ShareToChat
        open={dialog === "share"}
        onOpenChange={(open) => !open && setDialog(undefined)}
        share={{ url: `${window.location.origin}/school/rooms/${room.id}` }}
        title={room.title}
      />
    </SchoolPage>
  );
}

/** Host: the room's title, audience, timer and size. */
function RoomSettingsDialog({ room, onClose }: { room: school.StudyRoom; onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const mode = room.timer?.mode || school.StudyMode.POMODORO;
  const [form, setForm] = useState<RoomForm>({
    title: room.title,
    audience: room.audience,
    mode,
    memberCap: String(room.memberCap),
  });
  const save = useMutation({
    mutationFn: () =>
      roomClient().updateStudyRoom({
        roomId: room.id,
        title: form.title,
        audience: form.audience,
        memberCap: Number(form.memberCap) || room.memberCap,
        // Changing the kind of timer restarts its cycle, so it is only sent when it changed.
        mode: form.mode === mode ? school.StudyMode.UNSPECIFIED : form.mode,
      }),
    onSuccess: () => {
      refresh(queryClient, "rooms");
      onClose();
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("rooms.settings")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common:actions.cancel")}
          </Button>
          <Button
            loading={save.isPending}
            disabled={!form.title.trim()}
            onClick={() => save.mutate()}
          >
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <RoomFormFields form={form} onChange={setForm} />
        {form.mode !== mode && (
          <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm text-ink-soft">
            {t("rooms.modeChangeNote")}
          </p>
        )}
      </div>
    </Dialog>
  );
}

/** Host: invites friends and takes invitations back. */
function InviteDialog({ room, onClose }: { room: school.StudyRoom; onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const [chosen, setChosen] = useState<string[]>([]);
  const done = () => void queryClient.invalidateQueries({ queryKey: keys.room(room.id) });
  const invite = useMutation({
    mutationFn: () => roomClient().inviteToStudyRoom({ roomId: room.id, userIds: chosen }),
    onSuccess: () => {
      toast.success(t("rooms.invitedToast", { count: chosen.length }));
      setChosen([]);
      done();
    },
    onError: toastError,
  });
  const uninvite = useMutation({
    mutationFn: (userId: string) => roomClient().uninviteFromStudyRoom({ roomId: room.id, userId }),
    onSuccess: done,
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("rooms.invite")}
      description={t("rooms.inviteLead")}
      footer={
        <Button
          loading={invite.isPending}
          disabled={chosen.length === 0}
          onClick={() => invite.mutate()}
        >
          {t("rooms.inviteAction")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        {room.invitees.length > 0 && (
          <div>
            <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("rooms.invitees")}</p>
            <ul className="flex flex-wrap gap-1.5">
              {room.invitees.map((user) => (
                <li key={user.id}>
                  <button
                    type="button"
                    onClick={() => uninvite.mutate(user.id)}
                    aria-label={t("rooms.uninvite", { name: user.displayName })}
                    className="flex h-8 items-center gap-1.5 rounded-full bg-brand-soft pr-2 pl-1 text-sm font-bold"
                  >
                    <Avatar user={user} size="xs" />
                    <span className="max-w-28 truncate">
                      <EmojiText text={user.displayName} />
                    </span>
                    <X className="size-3.5 text-ink-soft" />
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        <FriendPicker selected={chosen} onChange={setChosen} />
      </div>
    </Dialog>
  );
}
