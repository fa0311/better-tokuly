import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { community, fromDate, media } from "@tokuly/api";
import { can, communityKeys, imageUrl, placesLeft } from "@tokuly/core/community";
import { CalendarPlus, MapPin, Video } from "lucide-react";
import { lazy, Suspense, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { useAccountTimeZone, zonedMoment, zonedTime } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { eventClient, NS, useEvents } from "./api";
import { useWhen } from "./parts";
import type { LatLng } from "./place-picker";

const PlacePicker = lazy(() => import("./place-picker"));
const A = community.AttendanceStatus;

/** "行く" / "キャンセル待ち 2番目" … the viewer's own answer as a chip; nothing when they have not answered. */
export function MyAnswerChip({ event }: { event: community.CalendarEvent }) {
  const { t } = useTranslation(NS);
  const viewer = event.viewer;
  switch (viewer?.status) {
    case A.GOING:
      return <Chip className="bg-lime text-[#17122b]">{t("event.status.going")}</Chip>;
    case A.WAITLISTED:
      return (
        <Chip className="bg-sun text-[#17122b]">
          {t("event.status.waitlisted", { position: viewer.waitlistPosition })}
        </Chip>
      );
    case A.INTERESTED:
      return <Chip className="bg-sky text-[#17122b]">{t("event.status.interested")}</Chip>;
    default:
      return null;
  }
}

/** An event in a list: the date as a tear-off calendar leaf, then what, where and how full. */
export function EventCard({
  event,
  handle,
  communityName,
}: {
  event: community.CalendarEvent;
  handle: string;
  /** Shown in lists that mix communities. */
  communityName?: string;
}) {
  const { t, i18n } = useTranslation(NS);
  const when = useWhen();
  const zone = useAccountTimeZone();
  const startMs = Number(event.startsAt?.seconds ?? 0n) * 1000;
  const start = new Date(startMs);
  const left = placesLeft(event);
  const part = (options: Intl.DateTimeFormatOptions) =>
    new Intl.DateTimeFormat(i18n.language, { timeZone: zone, ...options }).format(start);
  return (
    <Link
      to="/g/$handle/events/$eventId"
      params={{ handle, eventId: event.id }}
      className={cn(
        "flex gap-3 rounded-card bg-surface p-4 transition-colors hover:bg-surface-2",
        event.cancelled && "opacity-70",
      )}
    >
      <div className="sticker-sm grid h-16 w-14 shrink-0 place-items-center rounded-2xl bg-surface text-center leading-none">
        <div>
          <div className="text-[11px] font-black text-pink">{part({ month: "short" })}</div>
          <div className="text-2xl font-black">{part({ day: "numeric" }).replace(/\D+$/, "")}</div>
          <div className="text-[11px] font-bold text-ink-soft">{part({ weekday: "short" })}</div>
        </div>
      </div>
      <div className="min-w-0 flex-1">
        {communityName && (
          <p className="truncate text-xs font-black text-brand">
            <EmojiText text={communityName} />
          </p>
        )}
        <p className={cn("break-words font-black leading-snug", event.cancelled && "line-through")}>
          <EmojiText text={event.title} />
        </p>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-2 text-sm text-ink-soft">
          <span>{when(startMs, { date: false })}</span>
          <span className="inline-flex min-w-0 items-center gap-1">
            {event.online ? <Video className="size-3.5" /> : <MapPin className="size-3.5" />}
            <span className="truncate">{event.online ? t("event.online") : event.place?.name}</span>
          </span>
        </p>
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {event.cancelled ? (
            <Chip className="bg-coral text-white">{t("event.cancelled")}</Chip>
          ) : (
            <>
              <MyAnswerChip event={event} />
              <Chip>
                {event.capacity > 0
                  ? t("event.goingOf", { count: event.goingCount, capacity: event.capacity })
                  : t("event.going", { count: event.goingCount })}
              </Chip>
              {left === 0 && <Chip className="bg-sun text-[#17122b]">{t("event.full")}</Chip>}
            </>
          )}
        </div>
      </div>
    </Link>
  );
}

/** The community's calendar: what is coming, what is over, and (for those who may) a way to add to it. */
export function EventsTab({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const [past, setPast] = useState(false);
  const [creating, setCreating] = useState(false);
  const events = useEvents(c.id, past);
  return (
    <div>
      <div className="flex flex-wrap items-center gap-2">
        <Tabs
          items={[
            { value: "upcoming", label: t("event.upcoming") },
            { value: "past", label: t("event.past") },
          ]}
          value={past ? "past" : "upcoming"}
          onChange={(next) => setPast(next === "past")}
        />
        {can(c, community.Permission.CREATE_EVENT) && (
          <Button
            size="sm"
            className="ml-auto"
            icon={<CalendarPlus className="size-4" />}
            onClick={() => setCreating(true)}
          >
            {t("event.create")}
          </Button>
        )}
      </div>
      <div className="mt-3 flex flex-col gap-2">
        {events.isPending ? (
          <Skeleton className="h-24" />
        ) : events.items?.length === 0 ? (
          <EmptyState
            emoji="🗓️"
            title={past ? t("event.emptyPast") : t("event.empty")}
            body={past ? undefined : t("event.emptyBody")}
          />
        ) : (
          <>
            {events.items?.map((event) => (
              <EventCard key={event.id} event={event} handle={c.handle} />
            ))}
            <LoadMore query={events} />
          </>
        )}
      </div>
      {creating && <EventFormDialog community={c} onClose={() => setCreating(false)} />}
    </div>
  );
}

const pad = (value: number) => String(value).padStart(2, "0");
const clock = (minute: number) => `${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`;
const minuteOf = (time: string) => {
  const [hours = 0, minutes = 0] = time.split(":").map(Number);
  return hours * 60 + minutes;
};

/** Creating or editing an event. Dates and times are on the account's clock. */
export function EventFormDialog({
  community: c,
  event,
  onClose,
}: {
  community: community.Community;
  /** The event to edit; a new one without it. */
  event?: community.CalendarEvent;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const zone = useAccountTimeZone();
  const initialStart = event?.startsAt
    ? zonedTime(Number(event.startsAt.seconds) * 1000, zone)
    : // Tomorrow evening: a sensible place to start from.
      { ...zonedTime(Date.now() + 24 * 3_600_000, zone), minute: 18 * 60 };
  const initialEnd = event?.endsAt
    ? zonedTime(Number(event.endsAt.seconds) * 1000, zone)
    : undefined;

  const [title, setTitle] = useState(event?.title ?? "");
  const [description, setDescription] = useState(event?.description ?? "");
  const [date, setDate] = useState(initialStart.date);
  const [time, setTime] = useState(clock(initialStart.minute));
  const [endDate, setEndDate] = useState(initialEnd?.date ?? "");
  const [endTime, setEndTime] = useState(initialEnd ? clock(initialEnd.minute) : "");
  const [online, setOnline] = useState(event?.online ?? false);
  const [onlineUrl, setOnlineUrl] = useState(event?.onlineUrl ?? "");
  const [placeName, setPlaceName] = useState(event?.place?.name ?? "");
  const [address, setAddress] = useState(event?.place?.address ?? "");
  const [pin, setPin] = useState<LatLng | undefined>(
    event?.place?.location
      ? { lat: event.place.location.lat, lng: event.place.location.lng }
      : undefined,
  );
  const [pinning, setPinning] = useState(Boolean(event?.place?.location));
  const [capacity, setCapacity] = useState(event?.capacity ? String(event.capacity) : "");
  const [cover, setCover] = useState<{ assetId: string; preview: string } | null | undefined>();
  const [uploading, setUploading] = useState(false);
  const file = useRef<HTMLInputElement>(null);

  const startMs = date && time ? zonedMoment(date, minuteOf(time), zone) : 0;
  const endMs = endDate && endTime ? zonedMoment(endDate, minuteOf(endTime), zone) : 0;
  const ready =
    Boolean(title.trim()) && startMs > 0 && (online || Boolean(placeName.trim())) && !uploading;

  const pickCover = async (picked: File) => {
    setUploading(true);
    try {
      const asset = await uploadAndProcess(picked, {
        purpose: media.AssetPurpose.COMMUNITY_BANNER,
      });
      setCover({ assetId: asset.id, preview: URL.createObjectURL(picked) });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: NS }));
    } finally {
      setUploading(false);
    }
  };

  const save = useMutation({
    mutationFn: async () => {
      const place = online
        ? undefined
        : { name: placeName.trim(), address: address.trim(), location: pinning ? pin : undefined };
      const shared = {
        title: title.trim(),
        description: description.trim(),
        startsAt: fromDate(new Date(startMs)),
        endsAt: endMs ? fromDate(new Date(endMs)) : undefined,
        place,
        onlineUrl: online ? onlineUrl.trim() : "",
        capacity: Math.max(0, Number(capacity) || 0),
      };
      if (event) {
        const response = await eventClient().updateEvent({
          eventId: event.id,
          ...shared,
          online: online ? true : undefined,
          coverAssetId: cover === undefined ? undefined : (cover?.assetId ?? ""),
        });
        return response.event;
      }
      const response = await eventClient().createEvent({
        communityId: c.id,
        ...shared,
        online,
        coverAssetId: cover?.assetId ?? "",
      });
      return response.event;
    },
    onSuccess: (saved) => {
      if (saved) queryClient.setQueryData(communityKeys.event(saved.id), saved);
      void queryClient.invalidateQueries({ queryKey: communityKeys.eventLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.myEvents });
      toast.success(event ? t("saved") : t("event.created"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });

  const coverUrl = cover === null ? undefined : (cover?.preview ?? imageUrl(event?.cover, 320));
  return (
    <Dialog
      open
      onOpenChange={(next) => !next && onClose()}
      title={event ? t("event.form.editTitle") : t("event.form.title")}
      footer={
        <Button loading={save.isPending} disabled={!ready} onClick={() => save.mutate()}>
          {event ? t("save") : t("event.create")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField
          label={t("event.form.name")}
          value={title}
          maxLength={100}
          placeholder={t("event.form.namePlaceholder")}
          onChange={(e) => setTitle(e.target.value)}
        />
        <div className="grid grid-cols-2 gap-3">
          <TextField
            label={t("event.form.date")}
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
          />
          <TextField
            label={t("event.form.time")}
            type="time"
            value={time}
            onChange={(e) => setTime(e.target.value)}
          />
          <TextField
            label={t("event.form.endDate")}
            type="date"
            value={endDate}
            min={date}
            onChange={(e) => setEndDate(e.target.value)}
          />
          <TextField
            label={t("event.form.endTime")}
            type="time"
            value={endTime}
            onChange={(e) => setEndTime(e.target.value)}
          />
        </div>
        <p className="-mt-2 px-1 text-xs text-ink-faint">{t("event.form.endHint")}</p>
        <SwitchRow
          label={t("event.form.online")}
          hint={t("event.form.onlineHint")}
          checked={online}
          onChange={setOnline}
        />
        {online ? (
          <TextField
            label={t("event.form.url")}
            type="url"
            inputMode="url"
            value={onlineUrl}
            placeholder="https://"
            onChange={(e) => setOnlineUrl(e.target.value)}
          />
        ) : (
          <>
            <TextField
              label={t("event.form.place")}
              value={placeName}
              maxLength={100}
              placeholder={t("event.form.placePlaceholder")}
              onChange={(e) => setPlaceName(e.target.value)}
            />
            <TextField
              label={t("event.form.address")}
              value={address}
              maxLength={200}
              onChange={(e) => setAddress(e.target.value)}
            />
            <SwitchRow label={t("event.form.pin")} checked={pinning} onChange={setPinning} />
            {pinning && (
              <Suspense fallback={<Skeleton className="h-56" />}>
                <PlacePicker value={pin} onChange={setPin} />
              </Suspense>
            )}
          </>
        )}
        <TextField
          label={t("event.form.capacity")}
          hint={t("event.form.capacityHint")}
          type="number"
          inputMode="numeric"
          min={0}
          max={100000}
          value={capacity}
          onChange={(e) => setCapacity(e.target.value)}
        />
        <TextArea
          label={t("event.form.description")}
          value={description}
          maxLength={3000}
          onChange={(e) => setDescription(e.target.value)}
        />
        <div>
          <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("event.form.cover")}</p>
          {coverUrl && (
            <img
              src={coverUrl}
              alt=""
              className="mb-2 aspect-[3/1] w-full rounded-2xl border-2 border-line object-cover"
            />
          )}
          <input
            ref={file}
            type="file"
            accept="image/*"
            hidden
            onChange={(e) => {
              const picked = e.target.files?.[0];
              if (picked) void pickCover(picked);
              e.target.value = "";
            }}
          />
          <div className="flex gap-2">
            <Button
              variant="secondary"
              size="sm"
              loading={uploading}
              onClick={() => file.current?.click()}
            >
              {coverUrl ? t("art.change") : t("art.choose")}
            </Button>
            {coverUrl && (
              <Button variant="ghost" size="sm" onClick={() => setCover(null)}>
                {t("art.remove")}
              </Button>
            )}
          </div>
        </div>
      </div>
    </Dialog>
  );
}
