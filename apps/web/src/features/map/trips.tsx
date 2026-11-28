import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { geo, toMillis } from "@tokuly/api";
import { decodePolyline } from "@tokuly/core/map";
import { Bike, Car, Footprints, Navigation, Plus, Search, TrainFront } from "lucide-react";
import { type ComponentType, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { formatDistance } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import {
  mapKeys,
  placeClient,
  tripClient,
  tripDestinationName,
  useActiveTrips,
  useNow,
} from "./data";
import { fitMapToPoints, RouteLine, useMap } from "./map-view";
import { DestinationMarker } from "./pins";
import { MapSheet, SheetEmpty, SheetSection } from "./sheet";
import { useMapUi } from "./ui-store";

export const TRAVEL_MODES: {
  mode: geo.TravelMode;
  key: string;
  icon: ComponentType<{ className?: string }>;
}[] = [
  { mode: geo.TravelMode.WALK, key: "walk", icon: Footprints },
  { mode: geo.TravelMode.BICYCLE, key: "bicycle", icon: Bike },
  { mode: geo.TravelMode.DRIVE, key: "drive", icon: Car },
  { mode: geo.TravelMode.TRANSIT, key: "transit", icon: TrainFront },
];

export type TripDestination =
  | { kind: "friend"; userId: string; name: string }
  | { kind: "place"; placeId: string; name: string }
  | { kind: "point"; lat: number; lng: number; name: string };

function toDestination(destination: TripDestination) {
  switch (destination.kind) {
    case "friend":
      return {
        target: { case: "userId" as const, value: destination.userId },
        name: destination.name,
      };
    case "place":
      return {
        target: { case: "placeId" as const, value: destination.placeId },
        name: destination.name,
      };
    default:
      return {
        target: {
          case: "position" as const,
          value: { lat: destination.lat, lng: destination.lng },
        },
        name: destination.name,
      };
  }
}

/** "あと8分" and the clock time of arrival. */
function useEta(): (trip: geo.Trip, now: number) => { left: string; clock: string } | null {
  const { t, i18n } = useTranslation("map");
  return (trip, now) => {
    const eta = toMillis(trip.eta);
    if (!eta) return null;
    const minutes = Math.max(0, Math.round((eta - now) / 60_000));
    const left =
      minutes < 1
        ? t("trips.almost")
        : minutes < 60
          ? t("trips.minutesLeft", { count: minutes })
          : t("trips.hoursLeft", { hours: Math.floor(minutes / 60), minutes: minutes % 60 });
    return {
      left,
      clock: new Intl.DateTimeFormat(i18n.language, { hour: "2-digit", minute: "2-digit" }).format(
        eta,
      ),
    };
  };
}

/**
 * Starts a trip ("on my way"). With a `destination` it only asks how; without
 * one it first lets the person search for a place.
 */
export function StartTripDialog({
  open,
  onOpenChange,
  destination: fixed,
  initialMode = geo.TravelMode.WALK,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  destination?: TripDestination;
  initialMode?: geo.TravelMode;
}) {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const own = useLocations((state) => state.own);
  const [picked, setPicked] = useState<TripDestination | null>(null);
  const [mode, setMode] = useState(initialMode);
  const [notify, setNotify] = useState(true);
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const destination = fixed ?? picked;

  const search = useQuery({
    queryKey: [
      "geo",
      "placeSearch",
      submitted,
      own ? own.lat.toFixed(2) : "",
      own ? own.lng.toFixed(2) : "",
    ],
    queryFn: async () =>
      (
        await placeClient().searchPlaces({
          query: submitted,
          near: own ? { lat: own.lat, lng: own.lng } : undefined,
          limit: 8,
        })
      ).results,
    enabled: open && !fixed && submitted.length > 0,
    staleTime: 60_000,
  });

  const start = useMutation({
    mutationFn: (target: TripDestination) =>
      tripClient().startTrip({ destination: toDestination(target), mode, notifyOnArrival: notify }),
    onSuccess: (response) => {
      if (response.trip) {
        const trip = response.trip;
        queryClient.setQueryData<geo.Trip[]>(mapKeys.trips, (list) => [
          ...(list ?? []).filter((entry) => entry.user?.id !== trip.user?.id),
          trip,
        ]);
      }
      toast.success(t("trips.started"));
      onOpenChange(false);
      setPicked(null);
      useMapUi.getState().open({ kind: "trips" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={destination ? t("trips.startTo", { name: destination.name }) : t("trips.whereTo")}
      description={t("trips.startLead")}
      footer={
        destination && (
          <Button
            onClick={() => start.mutate(destination)}
            loading={start.isPending}
            icon={<Navigation className="size-4" />}
          >
            {t("trips.start")}
          </Button>
        )
      }
    >
      {!destination ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(query.trim());
          }}
        >
          <TextField
            label={t("trips.searchLabel")}
            placeholder={t("trips.searchPlaceholder")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            maxLength={80}
            trailing={
              <button
                type="submit"
                aria-label={t("trips.search")}
                className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
              >
                <Search className="size-4" />
              </button>
            }
          />
          <ul className="mt-3 space-y-1">
            {search.isFetching && <Skeleton className="h-12" />}
            {search.data?.length === 0 && (
              <li className="py-4 text-center text-sm text-ink-soft">{t("trips.searchEmpty")}</li>
            )}
            {search.data?.map((result) => (
              <li key={`${result.place?.id}${result.name}${result.position?.lat}`}>
                <button
                  type="button"
                  onClick={() =>
                    setPicked(
                      result.place
                        ? { kind: "place", placeId: result.place.id, name: result.place.name }
                        : {
                            kind: "point",
                            lat: result.position?.lat ?? 0,
                            lng: result.position?.lng ?? 0,
                            name: result.name,
                          },
                    )
                  }
                  className="flex w-full items-center gap-3 rounded-2xl px-3 py-2 text-left hover:bg-surface-2"
                >
                  <span className="text-xl" aria-hidden>
                    {result.place?.emoji || "📍"}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-bold">
                      {result.place?.name || result.name}
                    </span>
                    <span className="block truncate text-xs text-ink-soft">{result.address}</span>
                  </span>
                  {result.distanceMeters !== undefined && (
                    <span className="shrink-0 text-xs font-bold text-ink-soft">
                      {formatDistance(result.distanceMeters)}
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        </form>
      ) : (
        <div className="space-y-4">
          {!fixed && (
            <button
              type="button"
              onClick={() => setPicked(null)}
              className="text-sm font-bold text-brand"
            >
              ← {t("trips.changeDestination")}
            </button>
          )}
          <div className="grid grid-cols-4 gap-2" role="radiogroup" aria-label={t("travel.mode")}>
            {TRAVEL_MODES.map(({ mode: option, key, icon: Icon }) => (
              // biome-ignore lint/a11y/useSemanticElements: big tappable cards, not radio inputs.
              <button
                key={key}
                type="button"
                role="radio"
                aria-checked={mode === option}
                onClick={() => setMode(option)}
                className={cn(
                  "pressable flex flex-col items-center gap-1 rounded-2xl py-3 text-xs font-black",
                  mode === option ? "sticker-sm bg-brand text-on-brand" : "bg-surface-2",
                )}
              >
                <Icon className="size-5" />
                {t(`travel.${key}`)}
              </button>
            ))}
          </div>
          <SwitchRow
            label={t("trips.notify")}
            hint={t("trips.notifyHint")}
            checked={notify}
            onChange={setNotify}
          />
        </div>
      )}
    </Dialog>
  );
}

/** The routes and destinations of the trips I may see, drawn on the map. */
export function TripLayer() {
  const { data: trips } = useActiveTrips();
  const myId = useMe()?.user?.id;
  return (
    <>
      {(trips ?? []).map((trip) => {
        const mine = trip.user?.id === myId;
        const destination = trip.destination?.resolved;
        return (
          <TripOnMap
            key={trip.id}
            id={trip.id}
            polyline={trip.routePolyline}
            mine={mine}
            destination={
              destination ? { ...destination, name: tripDestinationName(trip) } : undefined
            }
          />
        );
      })}
    </>
  );
}

function TripOnMap({
  id,
  polyline,
  mine,
  destination,
}: {
  id: string;
  polyline: string;
  mine: boolean;
  destination?: { lat: number; lng: number; name: string };
}) {
  return (
    <>
      {polyline && (
        <RouteLine
          id={`trip-${id}`}
          polyline={polyline}
          color={mine ? "#7c5cff" : "#38bdf8"}
          dashed={!mine}
        />
      )}
      {destination && (
        <DestinationMarker lat={destination.lat} lng={destination.lng} label={destination.name} />
      )}
    </>
  );
}

/** Chips above the map for the trips that concern me: my own, and friends on their way to me. */
export function TripChips() {
  const { t } = useTranslation("map");
  const { data: trips } = useActiveTrips();
  const myId = useMe()?.user?.id;
  const now = useNow(20_000);
  const eta = useEta();
  const open = useMapUi((state) => state.open);
  const relevant = (trips ?? []).filter(
    (trip) =>
      trip.user?.id === myId ||
      (trip.destination?.target.case === "userId" && trip.destination.target.value === myId),
  );
  return (
    <>
      {relevant.map((trip) => {
        const mine = trip.user?.id === myId;
        const time = eta(trip, now);
        return (
          <button
            key={trip.id}
            type="button"
            data-testid="trip-chip"
            onClick={() => open({ kind: "trips" })}
            className="glass sticker-sm pressable flex h-9 max-w-full items-center gap-1.5 rounded-full px-3 text-xs font-black"
          >
            <Navigation className="size-3.5 shrink-0 text-brand" />
            <span className="truncate">
              {mine
                ? t("trips.chipMine", { place: tripDestinationName(trip) })
                : t("trips.chipComing", { name: trip.user?.displayName })}
              {time && ` · ${time.left}`}
            </span>
          </button>
        );
      })}
    </>
  );
}

/** The list of trips: mine with its controls, friends' with their ETA. */
export function TripsPanel() {
  const { t } = useTranslation("map");
  const map = useMap();
  const queryClient = useQueryClient();
  const { data: trips, isPending } = useActiveTrips();
  const myId = useMe()?.user?.id;
  const now = useNow(20_000);
  const eta = useEta();
  const { stack, back, close } = useMapUi();
  const [starting, setStarting] = useState(false);
  const [ending, setEnding] = useState<geo.Trip | null>(null);

  const end = useMutation({
    mutationFn: (trip: geo.Trip) => tripClient().endTrip({ tripId: trip.id }),
    onSuccess: (_response, trip) => {
      queryClient.setQueryData<geo.Trip[]>(mapKeys.trips, (list) =>
        (list ?? []).filter((entry) => entry.id !== trip.id),
      );
      toast.success(t("trips.ended"));
      setEnding(null);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  const show = (trip: geo.Trip) => {
    const points = decodePolyline(trip.routePolyline).map(([lng, lat]) => ({ lat, lng }));
    const traveller =
      trip.user?.id === myId
        ? useLocations.getState().own
        : useLocations.getState().friends[trip.user?.id ?? ""]?.position;
    if (traveller) points.push({ lat: traveller.lat, lng: traveller.lng });
    if (trip.destination?.resolved) points.push(trip.destination.resolved);
    fitMapToPoints(map, points);
  };

  const mine = (trips ?? []).filter((trip) => trip.user?.id === myId);
  const others = (trips ?? []).filter((trip) => trip.user?.id !== myId);

  const card = (trip: geo.Trip) => {
    const time = eta(trip, now);
    const isMine = trip.user?.id === myId;
    const mode = TRAVEL_MODES.find((entry) => entry.mode === trip.mode) ?? TRAVEL_MODES[0];
    const Icon = mode?.icon ?? Footprints;
    return (
      <li key={trip.id} data-testid="trip-card" className="rounded-2xl bg-surface-2 p-3">
        <button
          type="button"
          onClick={() => show(trip)}
          className="flex w-full items-center gap-3 text-left"
        >
          <Avatar user={trip.user} size="md" outlined />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-black">
              {isMine
                ? t("trips.toPlace", { place: tripDestinationName(trip) })
                : t("trips.friendTo", {
                    name: trip.user?.displayName,
                    place: tripDestinationName(trip),
                  })}
            </span>
            <span className="flex items-center gap-1.5 text-xs font-bold text-ink-soft">
              <Icon className="size-3.5" />
              {time ? t("trips.eta", { left: time.left, clock: time.clock }) : t("trips.noEta")}
              {trip.distanceRemainingMeters > 0 && (
                <span>· {formatDistance(trip.distanceRemainingMeters)}</span>
              )}
            </span>
          </span>
        </button>
        {isMine && (
          <div className="mt-2 flex items-center justify-between gap-2">
            <span className="text-[11px] font-bold text-ink-faint">
              {trip.sharedWithUserIds.length
                ? t("trips.sharedWithSome", { count: trip.sharedWithUserIds.length })
                : t("trips.sharedWithAll")}
            </span>
            <Button size="sm" variant="secondary" onClick={() => setEnding(trip)}>
              {t("trips.end")}
            </Button>
          </div>
        )}
      </li>
    );
  };

  return (
    <MapSheet
      testId="trips-panel"
      title={t("trips.title")}
      subtitle={t("trips.subtitle")}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      {isPending ? (
        <Skeleton className="h-20" />
      ) : (
        <>
          <SheetSection title={t("trips.mine")}>
            {mine.length ? (
              <ul className="space-y-2">{mine.map(card)}</ul>
            ) : (
              <Button
                block
                variant="lime"
                icon={<Plus className="size-4" />}
                onClick={() => setStarting(true)}
              >
                {t("trips.new")}
              </Button>
            )}
          </SheetSection>
          <SheetSection title={t("trips.friends")}>
            {others.length ? (
              <ul className="space-y-2">{others.map(card)}</ul>
            ) : (
              <SheetEmpty emoji="🧭" title={t("trips.emptyTitle")} body={t("trips.emptyBody")} />
            )}
          </SheetSection>
        </>
      )}
      <StartTripDialog open={starting} onOpenChange={setStarting} />
      <ConfirmDialog
        open={ending !== null}
        onOpenChange={(open) => !open && setEnding(null)}
        title={t("trips.endTitle")}
        description={t("trips.endBody")}
        confirmLabel={t("trips.end")}
        loading={end.isPending}
        onConfirm={() => ending && end.mutate(ending)}
      />
    </MapSheet>
  );
}
