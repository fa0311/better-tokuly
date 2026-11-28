import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { create, geo, toMillis } from "@tokuly/api";
import { pinState } from "@tokuly/core/map";
import { BellPlus, Check, Siren, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { useLocations } from "#/stores/locations";
import { useFlyToUser } from "./camera";
import {
  alertPlaceName,
  friendName,
  mapKeys,
  placeClient,
  safetyClient,
  useActiveSos,
  useFriendsById,
  useNow,
  usePlaceAlerts,
} from "./data";
import { MapSheet, SheetEmpty, SheetSection } from "./sheet";
import { useMapUi } from "./ui-store";

/** How long the SOS button must be held before the countdown starts. */
const HOLD_MS = 1200;
/** Seconds to change one's mind before the SOS goes out. */
const COUNTDOWN_SECONDS = 5;

type Where = "home" | "work" | "school" | "theirSpot" | "mySpot" | `place:${string}`;

/**
 * "Tell me when A arrives at / leaves B". With `subjectId` the friend is
 * fixed (opened from their sheet); otherwise the person picks one.
 */
export function CreateAlertDialog({
  open,
  onOpenChange,
  subjectId,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  subjectId?: string;
}) {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const friends = useFriendsById();
  const own = useLocations((state) => state.own);
  const locations = useLocations((state) => state.friends);
  const [chosen, setChosen] = useState("");
  const [where, setWhere] = useState<Where>("home");
  const [trigger, setTrigger] = useState<"arrive" | "leave">("arrive");
  const [recurring, setRecurring] = useState(false);
  const subject = subjectId ?? chosen;
  const theirs = locations[subject];
  const theirSpotKnown =
    Boolean(theirs?.position) && pinState(theirs as geo.FriendLocation, Date.now()) === "live";

  const places = useQuery({
    queryKey: ["geo", "myPlaces"],
    queryFn: async () => (await placeClient().listMyPlaces({})).places,
    enabled: open,
    staleTime: 60_000,
  });

  const create = useMutation({
    mutationFn: () => {
      const base = {
        subjectUserId: subject,
        trigger: trigger === "arrive" ? geo.AlertTrigger.ARRIVE : geo.AlertTrigger.LEAVE,
        recurring,
      };
      if (where === "home" || where === "work" || where === "school") {
        const label =
          where === "home"
            ? geo.PlaceLabel.HOME
            : where === "work"
              ? geo.PlaceLabel.WORK
              : geo.PlaceLabel.SCHOOL;
        return safetyClient().createPlaceAlert({
          ...base,
          where: { case: "subjectPlaceLabel", value: label },
        });
      }
      if (where === "theirSpot" && theirs?.position) {
        return safetyClient().createPlaceAlert({
          ...base,
          where: { case: "center", value: theirs.position },
          placeName: theirs.placeName || theirs.locality || t("safety.thisSpot"),
        });
      }
      if (where === "mySpot" && own) {
        return safetyClient().createPlaceAlert({
          ...base,
          where: { case: "center", value: { lat: own.lat, lng: own.lng } },
          placeName: t("safety.mySpotName"),
        });
      }
      return safetyClient().createPlaceAlert({
        ...base,
        where: { case: "placeId", value: where.replace(/^place:/, "") },
      });
    },
    onSuccess: (response) => {
      void queryClient.invalidateQueries({ queryKey: mapKeys.alerts });
      toast.success(
        response.alert?.status === geo.AlertStatus.PENDING_APPROVAL
          ? t("safety.createdPending")
          : t("safety.created"),
      );
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  const friendOptions = Object.values(friends)
    .filter((friend) => friend.user)
    .map((friend) => ({ value: friend.user?.id ?? "", label: friendName(friend, friend.user) }));
  const whereOptions: { value: Where; label: string }[] = [
    { value: "home", label: t("safety.where.home") },
    { value: "school", label: t("safety.where.school") },
    { value: "work", label: t("safety.where.work") },
    ...(theirSpotKnown
      ? [{ value: "theirSpot" as const, label: t("safety.where.theirSpot") }]
      : []),
    ...(own ? [{ value: "mySpot" as const, label: t("safety.where.mySpot") }] : []),
    ...(places.data ?? []).map((place) => ({
      value: `place:${place.id}` as const,
      label: `${place.emoji} ${place.name}`.trim(),
    })),
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("safety.createTitle")}
      description={t("safety.createLead")}
      footer={
        <Button
          onClick={() => create.mutate()}
          loading={create.isPending}
          disabled={!subject}
          icon={<BellPlus className="size-4" />}
        >
          {t("safety.create")}
        </Button>
      }
    >
      <div className="space-y-4">
        {!subjectId && (
          <SelectField
            label={t("safety.who")}
            value={chosen}
            onChange={setChosen}
            options={[{ value: "", label: t("safety.pickFriend") }, ...friendOptions]}
          />
        )}
        <SelectField<Where>
          label={t("safety.whereLabel")}
          value={where}
          onChange={setWhere}
          options={whereOptions}
          hint={
            where === "home" || where === "work" || where === "school"
              ? t("safety.labelHint")
              : undefined
          }
        />
        <div>
          <p className="mb-1.5 px-1 text-sm font-bold">{t("safety.when")}</p>
          <Tabs<"arrive" | "leave">
            items={[
              { value: "arrive", label: t("safety.arrive") },
              { value: "leave", label: t("safety.leave") },
            ]}
            value={trigger}
            onChange={setTrigger}
          />
        </div>
        <SwitchRow
          label={t("safety.recurring")}
          hint={t("safety.recurringHint")}
          checked={recurring}
          onChange={setRecurring}
        />
      </div>
    </Dialog>
  );
}

function AlertRow({ alert, mine }: { alert: geo.PlaceAlert; mine: boolean }) {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const [deleting, setDeleting] = useState(false);
  const refresh = () => queryClient.invalidateQueries({ queryKey: mapKeys.alerts });
  const respond = useMutation({
    mutationFn: (approve: boolean) =>
      safetyClient().respondPlaceAlert({ alertId: alert.id, approve }),
    onSuccess: (_response, approve) => {
      void refresh();
      toast.success(approve ? t("safety.approved") : t("safety.declined"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });
  const remove = useMutation({
    mutationFn: () => safetyClient().deletePlaceAlert({ alertId: alert.id }),
    onSuccess: () => {
      void refresh();
      setDeleting(false);
      toast.success(t("safety.deleted"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  const other = mine ? alert.subject : alert.owner;
  const place = alertPlaceName(alert);
  const leaving = alert.trigger === geo.AlertTrigger.LEAVE;
  const sentence = mine
    ? t(leaving ? "safety.mineLeave" : "safety.mineArrive", { name: other?.displayName, place })
    : t(leaving ? "safety.aboutMeLeave" : "safety.aboutMeArrive", {
        name: other?.displayName,
        place,
      });
  const statusKey =
    {
      [geo.AlertStatus.PENDING_APPROVAL]: "pending",
      [geo.AlertStatus.ACTIVE]: "active",
      [geo.AlertStatus.DECLINED]: "declined",
      [geo.AlertStatus.FIRED]: "fired",
      [geo.AlertStatus.EXPIRED]: "expired",
    }[alert.status as number] ?? "active";
  const pendingForMe = !mine && alert.status === geo.AlertStatus.PENDING_APPROVAL;

  return (
    <li data-testid="alert-row" className="rounded-2xl bg-surface-2 p-3">
      <div className="flex items-start gap-3">
        <Avatar user={other} size="sm" />
        <div className="min-w-0 flex-1">
          <p className="text-sm leading-snug font-bold">{sentence}</p>
          <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] font-bold text-ink-soft">
            <span
              className={cn(
                "rounded-full px-2 py-0.5",
                statusKey === "active"
                  ? "bg-mint/50 text-ink"
                  : statusKey === "pending"
                    ? "bg-sun/60 text-[#17122b]"
                    : "bg-surface-3",
              )}
            >
              {t(`safety.status.${statusKey}`)}
            </span>
            <span>{alert.recurring ? t("safety.everyTime") : t("safety.once")}</span>
            {toMillis(alert.lastFiredAt) > 0 && (
              <span>{t("safety.lastFired", { age: compactAge(toMillis(alert.lastFiredAt)) })}</span>
            )}
          </p>
        </div>
        <button
          type="button"
          onClick={() => setDeleting(true)}
          aria-label={t("safety.delete")}
          className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3"
        >
          <Trash2 className="size-4" />
        </button>
      </div>
      {pendingForMe && (
        <div className="mt-2 flex justify-end gap-2">
          <Button
            size="sm"
            variant="secondary"
            icon={<X className="size-4" />}
            loading={respond.isPending && respond.variables === false}
            onClick={() => respond.mutate(false)}
          >
            {t("safety.decline")}
          </Button>
          <Button
            size="sm"
            variant="lime"
            icon={<Check className="size-4" />}
            loading={respond.isPending && respond.variables === true}
            onClick={() => respond.mutate(true)}
          >
            {t("safety.approve")}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("safety.deleteTitle")}
        description={sentence}
        confirmLabel={t("safety.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </li>
  );
}

/**
 * The SOS button: hold it, then a countdown you can still cancel, then every
 * friend gets your exact live position — whatever ghost mode says — until you
 * resolve it.
 */
function SosTrigger() {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState(0);
  const [counting, setCounting] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const hold = useRef<{ startedAt: number; frame: number } | null>(null);

  const trigger = useMutation({
    mutationFn: () => safetyClient().triggerSos({ message: message.trim() }),
    onSuccess: (response) => {
      // The banner goes up from what the server answered, not from a second request that might fail.
      queryClient.setQueryData<geo.GetActiveSosResponse>(mapKeys.sos, (current) =>
        create(geo.GetActiveSosResponseSchema, {
          friends: current?.friends ?? [],
          mine: response.sos,
        }),
      );
      void queryClient.invalidateQueries({ queryKey: mapKeys.sos });
      setCounting(null);
      setMessage("");
    },
    // A failed SOS stays on screen (see the dialog): nobody may think help is on the way when it is not.
  });
  const cancel = () => {
    setCounting(null);
    trigger.reset();
  };

  const stopHold = () => {
    if (hold.current) cancelAnimationFrame(hold.current.frame);
    hold.current = null;
    setProgress(0);
  };
  const startHold = () => {
    if (hold.current) return;
    const startedAt = performance.now();
    const tick = (time: number) => {
      const done = Math.min(1, (time - startedAt) / HOLD_MS);
      setProgress(done);
      if (done >= 1) {
        stopHold();
        navigator.vibrate?.(80);
        setCounting(COUNTDOWN_SECONDS);
        return;
      }
      if (hold.current) hold.current.frame = requestAnimationFrame(tick);
    };
    hold.current = { startedAt, frame: requestAnimationFrame(tick) };
  };
  useEffect(() => stopHold, []);

  // The countdown: one step a second, then it goes out.
  useEffect(() => {
    if (counting === null || trigger.isPending) return;
    if (counting <= 0) {
      trigger.mutate();
      return;
    }
    const timer = setTimeout(
      () => setCounting((value) => (value === null ? null : value - 1)),
      1000,
    );
    return () => clearTimeout(timer);
  }, [counting]);

  return (
    <>
      <button
        type="button"
        data-testid="sos-hold"
        onPointerDown={startHold}
        onPointerUp={stopHold}
        onPointerLeave={stopHold}
        onPointerCancel={stopHold}
        onKeyDown={(event) =>
          (event.key === " " || event.key === "Enter") && !event.repeat && startHold()
        }
        onKeyUp={stopHold}
        // Tabbing away with the key still down must not finish the hold.
        onBlur={stopHold}
        onContextMenu={(event) => event.preventDefault()}
        className="relative flex h-16 w-full touch-none items-center justify-center gap-2 overflow-hidden rounded-2xl border-2 border-danger bg-danger/10 font-black text-danger select-none"
      >
        <span
          className="absolute inset-y-0 left-0 bg-danger/30"
          style={{ width: `${progress * 100}%` }}
          aria-hidden
        />
        <Siren className="relative size-5" />
        <span className="relative">{progress > 0 ? t("sos.keepHolding") : t("sos.hold")}</span>
      </button>
      <p className="mt-1.5 text-[11px] leading-relaxed font-bold text-ink-faint">
        {t("sos.explain")}
      </p>

      <Dialog
        open={counting !== null}
        onOpenChange={(open) => !open && !trigger.isPending && cancel()}
        title={trigger.isError ? t("sos.failedTitle") : t("sos.countdownTitle")}
        description={trigger.isError ? undefined : t("sos.countdownBody")}
        footer={
          <>
            <Button variant="secondary" onClick={cancel} disabled={trigger.isPending}>
              {t("sos.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => (counting === 0 ? trigger.mutate() : setCounting(0))}
              loading={trigger.isPending}
            >
              {trigger.isError ? t("sos.retry") : t("sos.sendNow")}
            </Button>
          </>
        }
      >
        {trigger.isError ? (
          <div
            role="alert"
            data-testid="sos-failed"
            className="rounded-2xl border-2 border-danger bg-danger/10 p-4 text-sm leading-relaxed font-bold text-danger"
          >
            <p className="font-black">{t("sos.failedBody")}</p>
            <p className="mt-1">{errorMessage(trigger.error, { namespace: "map" })}</p>
          </div>
        ) : (
          <p
            className="text-center text-7xl font-black text-danger tabular-nums"
            data-testid="sos-countdown"
            aria-live="assertive"
          >
            {Math.max(0, counting ?? 0)}
          </p>
        )}
        <TextField
          className="mt-4"
          label={t("sos.messageLabel")}
          placeholder={t("sos.messagePlaceholder")}
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          maxLength={200}
        />
      </Dialog>
    </>
  );
}

/** Resolving my own SOS. */
function useResolveSos() {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sosId: string) => safetyClient().resolveSos({ sosId }),
    onSuccess: () => {
      // The server said it is over: the banner comes down now, not after a second request.
      queryClient.setQueryData<geo.GetActiveSosResponse>(mapKeys.sos, (current) =>
        create(geo.GetActiveSosResponseSchema, { friends: current?.friends ?? [] }),
      );
      void queryClient.invalidateQueries({ queryKey: mapKeys.sos });
      toast.success(t("sos.resolved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });
}

/** Banners above the map: my own running SOS, and friends who need help. */
export function SosBanners() {
  const { t } = useTranslation("map");
  const { data } = useActiveSos();
  const resolve = useResolveSos();
  const flyToUser = useFlyToUser();
  const open = useMapUi((state) => state.open);
  const now = useNow(30_000);
  if (!data?.mine && !data?.friends.length) return null;
  return (
    <>
      {data.mine && (
        <div
          role="alert"
          data-testid="sos-mine"
          className="sticker pointer-events-auto flex w-full max-w-md items-center gap-3 rounded-2xl bg-danger px-4 py-2.5 text-white"
        >
          <Siren className="size-5 shrink-0 animate-wiggle" />
          <p className="min-w-0 flex-1 text-sm leading-snug font-black">
            {t("sos.active")}
            <span className="block text-xs font-bold opacity-90">
              {t("sos.activeBody", { count: data.mine.notifiedUserIds.length })}
            </span>
          </p>
          <Button
            size="sm"
            variant="secondary"
            loading={resolve.isPending}
            onClick={() => data.mine && resolve.mutate(data.mine.id)}
          >
            {t("sos.resolve")}
          </Button>
        </div>
      )}
      {data.friends.map((sos) => (
        <button
          key={sos.id}
          type="button"
          data-testid="sos-friend"
          onClick={() => {
            if (!sos.user) return;
            open({ kind: "friend", userId: sos.user.id });
            flyToUser(sos.user.id);
          }}
          className="sticker pressable pointer-events-auto flex w-full max-w-md items-center gap-3 rounded-2xl bg-danger px-4 py-2.5 text-left text-white"
        >
          <Avatar user={sos.user} size="sm" outlined />
          <span className="min-w-0 flex-1 text-sm leading-snug font-black">
            {t("sos.friendActive", { name: sos.user?.displayName })}
            <span className="block truncate text-xs font-bold opacity-90">
              {sos.message || t("sos.since", { age: compactAge(toMillis(sos.startedAt), now) })}
            </span>
          </span>
          <Siren className="size-5 shrink-0" />
        </button>
      ))}
    </>
  );
}

/** みまもり: SOS and place alerts. */
export function SafetyPanel() {
  const { t } = useTranslation("map");
  const { stack, back, close } = useMapUi();
  const alerts = usePlaceAlerts();
  const { data: sos } = useActiveSos();
  const resolve = useResolveSos();
  const [creating, setCreating] = useState(false);

  return (
    <MapSheet
      testId="safety-panel"
      title={t("safety.title")}
      subtitle={t("safety.subtitle")}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      <SheetSection title={t("sos.title")}>
        {sos?.mine ? (
          <div className="rounded-2xl border-2 border-danger bg-danger/10 p-3">
            <p className="text-sm font-black text-danger">{t("sos.active")}</p>
            <p className="mt-0.5 text-xs font-bold text-ink-soft">
              {t("sos.activeBody", { count: sos.mine.notifiedUserIds.length })}
            </p>
            <Button
              className="mt-2"
              block
              variant="secondary"
              loading={resolve.isPending}
              onClick={() => sos.mine && resolve.mutate(sos.mine.id)}
            >
              {t("sos.resolve")}
            </Button>
          </div>
        ) : (
          <SosTrigger />
        )}
      </SheetSection>

      <SheetSection
        title={t("safety.mine")}
        action={
          <Button
            size="sm"
            variant="lime"
            icon={<BellPlus className="size-4" />}
            onClick={() => setCreating(true)}
          >
            {t("safety.new")}
          </Button>
        }
      >
        {alerts.isPending ? (
          <Skeleton className="h-16" />
        ) : alerts.data?.mine.length ? (
          <ul className="space-y-2">
            {alerts.data.mine.map((alert) => (
              <AlertRow key={alert.id} alert={alert} mine />
            ))}
          </ul>
        ) : (
          <SheetEmpty emoji="🔔" title={t("safety.emptyTitle")} body={t("safety.emptyBody")} />
        )}
      </SheetSection>

      {Boolean(alerts.data?.aboutMe.length) && (
        <SheetSection title={t("safety.aboutMe")}>
          <ul className="space-y-2">
            {alerts.data?.aboutMe.map((alert) => (
              <AlertRow key={alert.id} alert={alert} mine={false} />
            ))}
          </ul>
        </SheetSection>
      )}
      <CreateAlertDialog open={creating} onOpenChange={setCreating} />
    </MapSheet>
  );
}
