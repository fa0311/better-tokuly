import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { school } from "@tokuly/api";
import { phaseKey, refreshSchool, schoolKeys, timerText } from "@tokuly/core/school";
import { Globe, Lock, Plus, Users } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { roomClient, toastError } from "./api";

import { Block, FriendPicker, RowsSkeleton, SchoolPage, Segmented, useNow } from "./parts";
import { setActiveSession } from "./session-store";
import { SubjectSelect } from "./subject-select";
import { syncClock, timerAt } from "./timer";

export function audienceKey(audience: school.RoomAudience): "public" | "friends" | "invited" {
  if (audience === school.RoomAudience.PUBLIC) return "public";
  return audience === school.RoomAudience.INVITED ? "invited" : "friends";
}

/** Open rooms of a scope. Their timers carry the server's clock, noted the moment they arrive. */
async function listRooms(scope: school.RoomScope): Promise<school.StudyRoom[]> {
  const { rooms } = await roomClient().listStudyRooms({ scope, page: { limit: 50 } });
  syncClock(rooms[0]?.timer);
  return rooms;
}

type RoomTab = "friends" | "public";

/** もくもく部屋: the room the caller is in, the rooms of friends and invitations, the public ones; opening a new one. */
export function RoomsScreen() {
  const { t } = useTranslation("school");
  const [tab, setTab] = useState<RoomTab>("friends");
  const [creating, setCreating] = useState(false);
  const scope = tab === "friends" ? school.RoomScope.FRIENDS : school.RoomScope.PUBLIC;
  const mine = useQuery({
    queryKey: schoolKeys.rooms(school.RoomScope.MINE),
    queryFn: () => listRooms(school.RoomScope.MINE),
    staleTime: 0,
  });
  const rooms = useQuery({
    queryKey: schoolKeys.rooms(scope),
    queryFn: () => listRooms(scope),
    staleTime: 0,
  });
  const mineIds = new Set((mine.data ?? []).map((room) => room.id));
  const others = (rooms.data ?? []).filter((room) => !mineIds.has(room.id));

  return (
    <SchoolPage
      section="rooms"
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("rooms.create")}
        </Button>
      }
    >
      <p className="mb-4 rounded-card bg-surface-2 px-4 py-3 text-sm text-ink-soft">
        {t("rooms.lead")}
      </p>
      {(mine.data ?? []).length > 0 && (
        <Block title={t("rooms.mine")}>
          <ul className="flex flex-col gap-3">
            {(mine.data ?? []).map((room) => (
              <RoomCard key={room.id} room={room} />
            ))}
          </ul>
        </Block>
      )}
      <Block
        title={t("rooms.open_")}
        action={
          <Tabs
            value={tab}
            onChange={(next: RoomTab) => setTab(next)}
            items={[
              { value: "friends", label: t("rooms.tabs.friends") },
              { value: "public", label: t("rooms.tabs.public") },
            ]}
          />
        }
      >
        {rooms.isPending ? (
          <RowsSkeleton rows={2} className="h-28" />
        ) : others.length === 0 ? (
          <EmptyState
            emoji="🪑"
            title={t(tab === "friends" ? "rooms.emptyFriends" : "rooms.emptyPublic")}
            body={t("rooms.emptyBody")}
            action={
              <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
                {t("rooms.create")}
              </Button>
            }
          />
        ) : (
          <ul className="flex flex-col gap-3">
            {others.map((room) => (
              <RoomCard key={room.id} room={room} />
            ))}
          </ul>
        )}
      </Block>
      {creating && <CreateRoomDialog onClose={() => setCreating(false)} />}
    </SchoolPage>
  );
}

/** A room in a list: its title and host, who is inside, and where its timer stands. */
function RoomCard({ room }: { room: school.StudyRoom }) {
  const { t } = useTranslation("school");
  const now = useNow(1000);
  const view = timerAt(room.timer, now);
  const audience = audienceKey(room.audience);
  const AudienceIcon = audience === "public" ? Globe : audience === "invited" ? Lock : Users;
  return (
    <li>
      <Link
        to="/school/rooms/$roomId"
        params={{ roomId: room.id }}
        className="pressable sticker flex flex-col gap-3 rounded-card bg-surface p-4"
      >
        <span className="flex items-start gap-3">
          <span className="min-w-0 flex-1">
            <span className="block truncate text-lg font-black">{room.title}</span>
            <span className="flex items-center gap-1.5 text-sm font-bold text-ink-soft">
              <Avatar user={room.host} size="xs" />
              <span className="truncate">
                <EmojiText text={room.host?.displayName ?? ""} />
              </span>
              <AudienceIcon
                className="size-3.5 shrink-0"
                aria-label={t(`rooms.audience.${audience}`)}
              />
            </span>
          </span>
          <span
            className={cn(
              "shrink-0 rounded-full px-2.5 py-1 text-xs font-black tabular-nums",
              view.state === school.TimerState.RUNNING
                ? "bg-brand text-on-brand"
                : "bg-surface-3 text-ink-soft",
            )}
          >
            {t(`timer.phase.${phaseKey(view)}`)}
            {view.state !== school.TimerState.IDLE &&
              ` ${timerText(view.mode === school.StudyMode.POMODORO ? Math.ceil(view.remainingMs / 1000) : view.elapsedMs / 1000)}`}
          </span>
        </span>
        <span className="flex items-center gap-2">
          <span className="flex -space-x-2">
            {room.members.slice(0, 6).map((member) => (
              <Avatar key={member.user?.id} user={member.user} size="sm" />
            ))}
          </span>
          <span className="text-sm font-bold text-ink-soft">
            {t("rooms.memberCount", { count: room.memberCount, cap: room.memberCap })}
          </span>
          <span className="ml-auto flex gap-1.5">
            {room.invited && !room.joined && (
              <span className="rounded-full bg-sun px-2 py-0.5 text-xs font-black text-[#17122b]">
                {t("rooms.invitedBadge")}
              </span>
            )}
            {room.joined && (
              <span className="rounded-full bg-lime px-2 py-0.5 text-xs font-black text-[#17122b]">
                {t("rooms.joinedBadge")}
              </span>
            )}
          </span>
        </span>
      </Link>
    </li>
  );
}

/** The shape of a room: its title, who may enter, the kind of timer, how many fit. Used to open one and to change one. */
export interface RoomForm {
  title: string;
  audience: school.RoomAudience;
  mode: school.StudyMode;
  memberCap: string;
}

export function RoomFormFields({
  form,
  onChange,
}: {
  form: RoomForm;
  onChange(form: RoomForm): void;
}) {
  const { t } = useTranslation("school");
  return (
    <>
      <TextField
        label={t("rooms.titleField")}
        value={form.title}
        maxLength={40}
        placeholder={t("rooms.titlePlaceholder")}
        onChange={(event) => onChange({ ...form, title: event.target.value })}
      />
      <div>
        <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("rooms.audienceLabel")}</p>
        <Segmented
          label={t("rooms.audienceLabel")}
          className="w-full"
          value={form.audience}
          onChange={(audience) => onChange({ ...form, audience })}
          options={[
            { value: school.RoomAudience.FRIENDS, label: t("rooms.audience.friends") },
            { value: school.RoomAudience.PUBLIC, label: t("rooms.audience.public") },
            { value: school.RoomAudience.INVITED, label: t("rooms.audience.invited") },
          ]}
        />
        <p className="px-1 pt-1.5 text-xs text-ink-faint">
          {t(`rooms.audienceHint.${audienceKey(form.audience)}`)}
        </p>
      </div>
      <div>
        <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("timer.mode")}</p>
        <Segmented
          label={t("timer.mode")}
          className="w-full"
          value={form.mode}
          onChange={(mode) => onChange({ ...form, mode })}
          options={[
            { value: school.StudyMode.POMODORO, label: t("timer.pomodoro") },
            { value: school.StudyMode.STOPWATCH, label: t("timer.stopwatch") },
          ]}
        />
      </div>
      <TextField
        label={t("rooms.cap")}
        hint={t("rooms.capHint")}
        type="number"
        inputMode="numeric"
        min={2}
        max={50}
        value={form.memberCap}
        onChange={(event) => onChange({ ...form, memberCap: event.target.value })}
      />
    </>
  );
}

function CreateRoomDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [form, setForm] = useState<RoomForm>({
    title: "",
    audience: school.RoomAudience.FRIENDS,
    mode: school.StudyMode.POMODORO,
    memberCap: "",
  });
  const [subjectId, setSubjectId] = useState("");
  const [invitees, setInvitees] = useState<string[]>([]);
  const create = useMutation({
    mutationFn: () =>
      roomClient().createStudyRoom({
        title: form.title,
        audience: form.audience,
        mode: form.mode,
        memberCap: Number(form.memberCap) || 0,
        subjectId,
        inviteUserIds: invitees,
      }),
    onSuccess: (response) => {
      setActiveSession(response.session, response.heartbeatIntervalSeconds);
      refreshSchool(queryClient, "rooms", "study");
      onClose();
      if (response.room)
        void navigate({ to: "/school/rooms/$roomId", params: { roomId: response.room.id } });
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("rooms.createTitle")}
      description={t("rooms.createLead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common:actions.cancel")}
          </Button>
          <Button
            loading={create.isPending}
            disabled={!form.title.trim()}
            onClick={() => create.mutate()}
          >
            {t("rooms.createAction")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <RoomFormFields form={form} onChange={setForm} />
        <SubjectSelect label={t("rooms.mySubject")} value={subjectId} onChange={setSubjectId} />
        <div>
          <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("rooms.inviteFriends")}</p>
          <FriendPicker selected={invitees} onChange={setInvitees} />
        </div>
      </div>
    </Dialog>
  );
}
