import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, community, errorReason } from "@tokuly/api";
import {
  can,
  checkInOpen,
  communityHref,
  communityKeys,
  eventHref,
  eventPhase,
  imageUrl,
  isMember,
  placesLeft,
  timeoutUntilMs,
} from "@tokuly/core/community";
import {
  Ban,
  CalendarClock,
  Check,
  ExternalLink,
  History,
  MapPin,
  MoreHorizontal,
  Pencil,
  QrCode,
  Star,
  Trash2,
  UserCheck,
  Video,
  X,
} from "lucide-react";
import { Suspense, useState } from "react";
import { toast } from "sonner";
import { onDemand } from "#/components/on-demand";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { CommentThread } from "#/features/comments";
import { EmojiText } from "#/features/emoji";
import { ReactionBar } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { eventClient, NS, useAttendees, useEvent } from "./api";
import { useCurrentCommunity } from "./context";
import { EventFormDialog, MyAnswerChip } from "./events";
import { ContentHistoryDialog, ReasonDialog } from "./mod-dialogs";
import { Notice, SectionTitle, useWhen } from "./parts";
import { useShareMenu } from "./share";

const StaticMapPreview = onDemand(
  () => import("#/features/map").then((module) => module.StaticMapPreview),
  "notice",
);

const A = community.AttendanceStatus;
const R = community.RsvpAnswer;

/** One event: what, when and where; the viewer's answer; who is coming; check-in. */
export function EventScreen({ eventId }: { eventId: string }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const query = useEvent(eventId);
  const event = query.data;
  const back = communityHref(c.handle);
  if (query.isPending)
    return (
      <Page title={t("event.title")} back={back}>
        <Skeleton className="h-64" />
      </Page>
    );
  if (!event || event.communityId !== c.id) {
    const reason = query.error ? errorReason(query.error) : "EVENT_NOT_FOUND";
    return (
      <Page title={t("event.title")} back={back}>
        <EmptyState
          emoji="🫥"
          title={
            !reason || reason === "EVENT_NOT_FOUND"
              ? t("event.gone")
              : errorMessage(query.error, { namespace: NS })
          }
        />
      </Page>
    );
  }
  return (
    <Page title={t("event.title")} back={back} actions={<EventActions event={event} />}>
      <EventBody event={event} />
    </Page>
  );
}

function EventBody({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const when = useWhen();
  const now = Date.now();
  const startMs = Number(event.startsAt?.seconds ?? 0n) * 1000;
  const endMs = Number(event.endsAt?.seconds ?? 0n) * 1000;
  const phase = eventPhase(event, now);
  const cover = imageUrl(event.cover, 640);
  const location = event.place?.location;
  const left = placesLeft(event);
  const subject = { type: common.SubjectType.COMMUNITY_EVENT, id: event.id };
  const sameDay = when(startMs, { time: false }) === when(endMs, { time: false });

  return (
    <>
      <article className="sticker overflow-hidden rounded-card bg-surface">
        {cover && <img src={cover} alt="" className="aspect-[3/1] w-full object-cover" />}
        <div className="p-4 sm:p-5">
          <div className="flex flex-wrap gap-1.5">
            {event.cancelled && <Chip className="bg-coral text-white">{t("event.cancelled")}</Chip>}
            {!event.cancelled && phase === "running" && (
              <Chip className="bg-lime text-[#17122b]">{t("event.running")}</Chip>
            )}
            {!event.cancelled && phase === "over" && <Chip>{t("event.over")}</Chip>}
            <MyAnswerChip event={event} />
            {event.viewer?.checkedInAt && (
              <Chip className="bg-mint text-[#17122b]">
                <Check className="size-3" />
                {t("checkIn.done")}
              </Chip>
            )}
          </div>
          <h2
            className={cn(
              "mt-2 break-words text-2xl font-black leading-snug",
              event.cancelled && "line-through",
            )}
          >
            <EmojiText text={event.title} />
          </h2>
          {event.cancelled && event.cancelReason && (
            <Notice tone="warn" className="mt-3">
              {t("event.cancelReason", { reason: event.cancelReason })}
            </Notice>
          )}
          <dl className="mt-4 flex flex-col gap-3 text-[15px]">
            <div className="flex gap-3">
              <dt>
                <CalendarClock
                  className="mt-0.5 size-5 text-ink-soft"
                  aria-label={t("event.when")}
                />
              </dt>
              <dd className="font-bold">
                {when(startMs)} – {sameDay ? when(endMs, { date: false }) : when(endMs)}
              </dd>
            </div>
            <div className="flex gap-3">
              <dt>
                {event.online ? (
                  <Video className="mt-0.5 size-5 text-ink-soft" aria-label={t("event.where")} />
                ) : (
                  <MapPin className="mt-0.5 size-5 text-ink-soft" aria-label={t("event.where")} />
                )}
              </dt>
              <dd className="min-w-0">
                {event.online ? (
                  <>
                    <p className="font-bold">{t("event.online")}</p>
                    {event.onlineUrl && /^https:\/\//.test(event.onlineUrl) ? (
                      <a
                        href={event.onlineUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-flex items-center gap-1 break-all text-sm font-bold text-brand"
                      >
                        {event.onlineUrl}
                        <ExternalLink className="size-3.5 shrink-0" />
                      </a>
                    ) : (
                      <p className="text-sm text-ink-soft">{t("event.onlineUrlHint")}</p>
                    )}
                  </>
                ) : (
                  <>
                    <p className="break-words font-bold">
                      <EmojiText text={event.place?.name ?? ""} />
                    </p>
                    {event.place?.address && (
                      <p className="break-words text-sm text-ink-soft">{event.place.address}</p>
                    )}
                  </>
                )}
              </dd>
            </div>
          </dl>
          {location && (
            <Suspense fallback={<Skeleton className="mt-3 h-40" />}>
              <StaticMapPreview
                lat={location.lat}
                lng={location.lng}
                label={event.place?.name}
                className="mt-3 h-40 rounded-2xl"
              />
            </Suspense>
          )}
          {event.description && (
            <p className="mt-4 whitespace-pre-wrap break-words leading-relaxed">
              <EmojiText text={event.description} />
            </p>
          )}
          {event.organizer && (
            <Link
              to="/u/$handle"
              params={{ handle: event.organizer.handle }}
              className="mt-4 flex items-center gap-2 text-sm"
            >
              <Avatar user={event.organizer} size="sm" />
              <span className="text-ink-soft">{t("event.organizer")}</span>
              <span className="truncate font-bold">
                <EmojiText text={event.organizer.displayName || event.organizer.handle} />
              </span>
            </Link>
          )}
          <div className="mt-4 flex flex-wrap gap-1.5">
            <Chip>
              {event.capacity > 0
                ? t("event.goingOf", { count: event.goingCount, capacity: event.capacity })
                : t("event.going", { count: event.goingCount })}
            </Chip>
            {left === 0 && <Chip className="bg-sun text-[#17122b]">{t("event.full")}</Chip>}
            {event.waitlistCount > 0 && (
              <Chip>{t("event.waitlist", { count: event.waitlistCount })}</Chip>
            )}
            {event.interestedCount > 0 && (
              <Chip>{t("event.interested", { count: event.interestedCount })}</Chip>
            )}
            {event.attendedCount > 0 && (
              <Chip>{t("event.attended", { count: event.attendedCount })}</Chip>
            )}
          </div>
          <RsvpBar event={event} />
          <ReactionBar subject={subject} className="mt-4" />
        </div>
      </article>

      <CheckIn event={event} />
      {isMember(c) && <Attendees event={event} />}

      <SectionTitle>{t("event.comments")}</SectionTitle>
      <CommentThread subject={subject} />
    </>
  );
}

/** The viewer's answer: going, interested, not going. A full event puts "going" on the waitlist. */
function RsvpBar({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const queryClient = useQueryClient();
  const now = Date.now();
  const rsvp = useMutation({
    mutationFn: (answer: community.RsvpAnswer) => eventClient().rsvp({ eventId: event.id, answer }),
    onSuccess: ({ event: next }) => {
      if (!next) return;
      queryClient.setQueryData(communityKeys.event(event.id), next);
      void queryClient.invalidateQueries({ queryKey: communityKeys.attendeeLists(event.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.eventLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.myEvents });
      const status = next.viewer?.status;
      if (status === A.WAITLISTED)
        toast(t("event.toast.waitlisted", { position: next.viewer?.waitlistPosition }), {
          icon: "⏳",
        });
      else if (status === A.GOING) toast.success(t("event.toast.going"));
      else if (status === A.INTERESTED) toast(t("event.toast.interested"), { icon: "⭐" });
      else toast(t("event.toast.notGoing"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (event.cancelled || eventPhase(event, now) === "over") return null;
  if (!isMember(c)) return <Notice className="mt-4">{t("event.joinFirst")}</Notice>;
  if (timeoutUntilMs(c.viewer, now) > 0)
    return <Notice className="mt-4">{t("reply.timedOut")}</Notice>;

  const status = event.viewer?.status ?? A.UNSPECIFIED;
  const wantsToGo = status === A.GOING || status === A.WAITLISTED;
  const full = placesLeft(event) === 0;
  const option = (
    answer: community.RsvpAnswer,
    active: boolean,
    label: string,
    icon: React.ReactNode,
  ) => (
    <Button
      variant={active ? "lime" : "secondary"}
      size="sm"
      aria-pressed={active}
      icon={icon}
      disabled={rsvp.isPending}
      onClick={() => !active && rsvp.mutate(answer)}
    >
      {label}
    </Button>
  );
  return (
    <div className="mt-4">
      <fieldset className="flex flex-wrap gap-2">
        <legend className="sr-only">{t("event.answer")}</legend>
        {option(
          R.GOING,
          wantsToGo,
          status === A.WAITLISTED
            ? t("event.rsvp.waitlisted")
            : full && !wantsToGo
              ? t("event.rsvp.joinWaitlist")
              : t("event.rsvp.going"),
          <Check className="size-4" />,
        )}
        {option(
          R.INTERESTED,
          status === A.INTERESTED,
          t("event.rsvp.interested"),
          <Star className="size-4" />,
        )}
        {option(
          R.NOT_GOING,
          status === A.NOT_GOING,
          t("event.rsvp.notGoing"),
          <X className="size-4" />,
        )}
      </fieldset>
      {status === A.WAITLISTED && (
        <p className="mt-2 text-sm text-ink-soft">
          {t("event.waitlistHint", { position: event.viewer?.waitlistPosition })}
        </p>
      )}
    </div>
  );
}

/** Check-in: the organiser shows a code that changes every minute; whoever is going types it in. */
function CheckIn({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [code, setCode] = useState("");
  const [showing, setShowing] = useState(false);
  const open = checkInOpen(event, Date.now());
  const viewer = event.viewer;
  const checkIn = useMutation({
    mutationFn: () => eventClient().checkIn({ eventId: event.id, code: code.trim() }),
    onSuccess: ({ event: next }) => {
      if (next) queryClient.setQueryData(communityKeys.event(event.id), next);
      setCode("");
      toast.success(t("checkIn.welcome"), { icon: "🎉" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (!open) return null;
  const going = viewer?.status === A.GOING;
  if (!viewer?.canManage && !(going && !viewer?.checkedInAt)) return null;
  return (
    <section className="mt-4 rounded-card bg-surface p-4">
      <h3 className="font-black">{t("checkIn.title")}</h3>
      {going && !viewer?.checkedInAt && (
        <form
          className="mt-2 flex items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (code.trim()) checkIn.mutate();
          }}
        >
          <TextField
            className="min-w-0 flex-1"
            label={t("checkIn.code")}
            hint={t("checkIn.codeHint")}
            value={code}
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={6}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
          />
          <Button
            type="submit"
            className="mb-6"
            loading={checkIn.isPending}
            disabled={code.length < 6}
          >
            {t("checkIn.action")}
          </Button>
        </form>
      )}
      {viewer?.canManage && (
        <Button
          className="mt-2"
          variant="secondary"
          icon={<QrCode className="size-4" />}
          onClick={() => setShowing(true)}
        >
          {t("checkIn.show")}
        </Button>
      )}
      {showing && <CheckInCodeDialog eventId={event.id} onClose={() => setShowing(false)} />}
    </section>
  );
}

function CheckInCodeDialog({ eventId, onClose }: { eventId: string; onClose(): void }) {
  const { t } = useTranslation(NS);
  const query = useQuery({
    queryKey: ["community", "check-in-code", eventId],
    queryFn: () => eventClient().getCheckInCode({ eventId }),
    staleTime: 0,
    gcTime: 0,
    // The code changes about once a minute: ask again when this one stops being shown.
    refetchInterval: (state) => {
      const expires = state.state.data?.expiresAt;
      if (!expires) return 15_000;
      return Math.max(1000, Number(expires.seconds) * 1000 - Date.now() + 500);
    },
  });
  return (
    <Dialog
      open
      onOpenChange={(next) => !next && onClose()}
      title={t("checkIn.showTitle")}
      description={t("checkIn.showBody")}
    >
      {query.error ? (
        <p className="text-sm text-danger">{errorMessage(query.error, { namespace: NS })}</p>
      ) : (
        <output
          aria-label={t("checkIn.code")}
          className="sticker mx-auto block w-fit rounded-card bg-lime px-8 py-6 text-center font-mono text-5xl font-black tracking-[0.2em] text-[#17122b]"
        >
          {query.data?.code ?? "······"}
        </output>
      )}
    </Dialog>
  );
}

const LISTS = [
  { value: "going", status: A.GOING },
  { value: "waitlisted", status: A.WAITLISTED },
  { value: "interested", status: A.INTERESTED },
] as const;

/** Who answered, by answer; the organiser and the staff also see who checked in and can check someone in by hand. */
function Attendees({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const queryClient = useQueryClient();
  const [list, setList] = useState<(typeof LISTS)[number]["value"]>("going");
  const status = LISTS.find((each) => each.value === list)?.status ?? A.GOING;
  const attendees = useAttendees(event.id, status);
  const manage = event.viewer?.canManage && checkInOpen(event, Date.now());
  const checkIn = useMutation({
    mutationFn: (userId: string) => eventClient().checkInAttendee({ eventId: event.id, userId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.attendeeLists(event.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.event(event.id) });
      toast.success(t("checkIn.byHandDone"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const counts = {
    going: event.goingCount,
    waitlisted: event.waitlistCount,
    interested: event.interestedCount,
  };
  return (
    <section className="mt-4">
      <SectionTitle>{t("event.attendees")}</SectionTitle>
      <Tabs<(typeof LISTS)[number]["value"]>
        items={LISTS.map(({ value }) => ({
          value,
          label: `${t(`event.list.${value}`)} ${counts[value]}`,
        }))}
        value={list}
        onChange={setList}
      />
      <div className="mt-2 rounded-card bg-surface p-1.5">
        {attendees.isPending ? (
          <Skeleton className="h-14" />
        ) : attendees.items?.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-ink-faint">{t("event.nobody")}</p>
        ) : (
          <>
            {attendees.items?.map((attendee) => (
              <ListRow
                key={attendee.user?.id}
                leading={<Avatar user={attendee.user} />}
                title={
                  <EmojiText text={attendee.user?.displayName || attendee.user?.handle || ""} />
                }
                subtitle={
                  attendee.checkedInAt
                    ? t("checkIn.at", {
                        time: when(Number(attendee.checkedInAt.seconds) * 1000, { date: false }),
                      })
                    : `@${attendee.user?.handle}`
                }
                trailing={
                  attendee.checkedInAt ? (
                    <Check className="size-5 text-success" aria-label={t("checkIn.done")} />
                  ) : manage && status === A.GOING ? (
                    <Button
                      variant="secondary"
                      size="sm"
                      icon={<UserCheck className="size-4" />}
                      disabled={checkIn.isPending}
                      onClick={() => attendee.user && checkIn.mutate(attendee.user.id)}
                    >
                      {t("checkIn.byHand")}
                    </Button>
                  ) : undefined
                }
              />
            ))}
            <LoadMore query={attendees} />
          </>
        )}
      </div>
    </section>
  );
}

function EventActions({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [history, setHistory] = useState(false);
  const share = useShareMenu({
    path: eventHref(c.handle, event.id),
    title: event.title,
    subject: { type: common.SubjectType.COMMUNITY_EVENT, id: event.id },
    share: c.visibility !== community.CommunityVisibility.HIDDEN,
    report: !event.viewer?.canManage,
  });
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: communityKeys.eventLists(c.id) });
    void queryClient.invalidateQueries({ queryKey: communityKeys.myEvents });
  };
  const cancel = useMutation({
    mutationFn: (reason: string) => eventClient().cancelEvent({ eventId: event.id, reason }),
    onSuccess: ({ event: next }) => {
      setCancelling(false);
      if (next) queryClient.setQueryData(communityKeys.event(event.id), next);
      refresh();
      toast(t("event.cancelDone"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const remove = useMutation({
    mutationFn: () => eventClient().deleteEvent({ eventId: event.id }),
    onSuccess: () => {
      setDeleting(false);
      refresh();
      toast(t("event.deleted"));
      void navigate({ to: "/g/$handle", params: { handle: c.handle }, search: { tab: "events" } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });

  const actions: MenuAction[] = [];
  if (event.viewer?.canManage) {
    if (!event.cancelled) {
      actions.push(
        { label: t("edit"), icon: <Pencil />, onSelect: () => setEditing(true) },
        { label: t("event.cancel"), icon: <Ban />, onSelect: () => setCancelling(true) },
      );
    }
  }
  if (can(c, community.Permission.MODERATE_CONTENT))
    actions.push({
      label: t("history.action"),
      icon: <History />,
      onSelect: () => setHistory(true),
    });
  actions.push(...share.actions);
  if (event.viewer?.canManage)
    actions.push({
      label: t("delete"),
      icon: <Trash2 />,
      danger: true,
      onSelect: () => setDeleting(true),
    });
  if (actions.length === 0) return null;
  return (
    <>
      <Menu
        trigger={
          <Button variant="ghost" size="icon-sm" aria-label={t("event.more")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={actions}
      />
      {share.sheets}
      {editing && <EventFormDialog community={c} event={event} onClose={() => setEditing(false)} />}
      <ReasonDialog
        open={cancelling}
        onOpenChange={setCancelling}
        title={t("event.cancelTitle")}
        description={t("event.cancelBody")}
        confirmLabel={t("event.cancel")}
        loading={cancel.isPending}
        onConfirm={(reason) => cancel.mutate(reason)}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("event.deleteTitle")}
        description={t("event.deleteBody")}
        confirmLabel={t("delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <ContentHistoryDialog
        content={history ? { case: "eventId", value: event.id } : undefined}
        onClose={() => setHistory(false)}
      />
    </>
  );
}
