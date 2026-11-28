import { type common, geo, toMillis } from "@tokuly/api";
import { circleRing, isBlurred, isMoving, pinState, stayMillis } from "@tokuly/core/map";
import {
  BatteryCharging,
  BatteryFull,
  BatteryLow,
  BatteryMedium,
  Briefcase,
  GraduationCap,
  House,
  Moon,
  Navigation,
  Siren,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { compactAge, compactDuration, kmh } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { useNow } from "./data";
import { type MapGeoJson, MapPin, useMap, useMapLayer } from "./map-view";

export const PLACE_ICONS = {
  [geo.PlaceLabel.HOME]: House,
  [geo.PlaceLabel.WORK]: Briefcase,
  [geo.PlaceLabel.SCHOOL]: GraduationCap,
} as const;

export const ACTIVITY_EMOJI: Record<number, string> = {
  [geo.Activity.WALKING]: "🚶",
  [geo.Activity.RUNNING]: "🏃",
  [geo.Activity.CYCLING]: "🚲",
  [geo.Activity.DRIVING]: "🚗",
  [geo.Activity.TRAIN]: "🚃",
  [geo.Activity.FLYING]: "✈️",
  [geo.Activity.BOAT]: "⛴️",
};

export const ACTIVITY_KEYS: Record<number, string> = {
  [geo.Activity.STILL]: "still",
  [geo.Activity.WALKING]: "walking",
  [geo.Activity.RUNNING]: "running",
  [geo.Activity.CYCLING]: "cycling",
  [geo.Activity.DRIVING]: "driving",
  [geo.Activity.TRAIN]: "train",
  [geo.Activity.FLYING]: "flying",
  [geo.Activity.BOAT]: "boat",
};

/** A value read from the map that re-renders only when it changes (zoom thresholds, bearing). */
export function useMapValue<T>(
  read: (map: ReturnType<typeof useMap>) => T,
  events: "move" | "zoom" | "rotate" | "rotate pitch" = "move",
): T {
  const map = useMap();
  const [value, setValue] = useState(() => read(map));
  useEffect(() => {
    const update = () => setValue(read(map));
    // The map may have moved between the first render and this subscription.
    update();
    for (const event of events.split(" ") as ("move" | "zoom" | "rotate" | "pitch")[])
      map.on(event, update);
    return () => {
      for (const event of events.split(" ") as ("move" | "zoom" | "rotate" | "pitch")[])
        map.off(event, update);
    };
  }, [map, events]);
  return value;
}

/** Battery as a small pill: a number friends can read at a glance. */
export function BatteryBadge({
  percent,
  charging,
  className,
}: {
  percent: number;
  charging: boolean;
  className?: string;
}) {
  const { t } = useTranslation("map");
  const low = percent <= 20 && !charging;
  const Icon = charging
    ? BatteryCharging
    : low
      ? BatteryLow
      : percent < 60
        ? BatteryMedium
        : BatteryFull;
  return (
    <span
      title={
        charging ? `${t("battery", { percent })} · ${t("charging")}` : t("battery", { percent })
      }
      data-testid="battery"
      className={cn(
        "sticker-sm flex h-6 items-center gap-0.5 rounded-full px-1.5 text-[10px] leading-none font-black",
        low ? "bg-danger text-white" : charging ? "bg-mint text-[#17122b]" : "bg-surface text-ink",
        className,
      )}
    >
      <Icon className="size-3.5" aria-hidden />
      {percent}
      <span className="sr-only">%{charging ? ` ${t("charging")}` : ""}</span>
    </span>
  );
}

/** The text under a pin: speed while moving, time on the spot, "around here", or age when stale. */
export function usePinCaption(
  location: geo.FriendLocation,
  now: number,
): { text: string; tone: "live" | "calm" | "stale" | "blurred" } {
  const { t } = useTranslation("map");
  const state = pinState(location, now);
  const recordedAt = toMillis(location.recordedAt);
  // A blurred pin keeps its dashed look whatever its age: it never turns into a point.
  const blurred = isBlurred(location);
  if (state === "paused") return { text: t("pin.paused"), tone: blurred ? "blurred" : "stale" };
  if (state === "stale")
    return { text: compactAge(recordedAt, now), tone: blurred ? "blurred" : "stale" };
  if (blurred) return { text: t("pin.around"), tone: "blurred" };
  if (isMoving(location, now))
    return {
      text: `${ACTIVITY_EMOJI[location.activity] ?? ""} ${kmh(location.speedMps)} km/h`.trim(),
      tone: "live",
    };
  const stay = stayMillis(location, now);
  if (stay !== null) return { text: compactDuration(stay), tone: "calm" };
  return { text: compactAge(recordedAt, now), tone: "live" };
}

export interface FriendMarkerProps {
  user: common.UserSummary;
  name: string;
  location: geo.FriendLocation;
  focused: boolean;
  /** They sent an SOS that is still running. */
  sos?: boolean;
  showName: boolean;
  offset?: [number, number];
  onSelect(): void;
}

/**
 * A friend on the map: their face as a sticker with a tail, plus the little
 * signals around it (battery, place, sleep, speed, how long they have been
 * there). A blurred pin is drawn dashed, a stale one faded.
 */
export function FriendMarker({
  user,
  name,
  location,
  focused,
  sos,
  showName,
  offset,
  onSelect,
}: FriendMarkerProps) {
  const now = useNow(10_000);
  const caption = usePinCaption(location, now);
  if (!location.position) return null;
  const state = pinState(location, now);
  const faded = state === "stale" || state === "paused";
  const moving = isMoving(location, now);
  const blurred = isBlurred(location);
  const PlaceIcon = blurred
    ? undefined
    : PLACE_ICONS[location.placeLabel as keyof typeof PLACE_ICONS];

  return (
    <MapPin
      lng={location.position.lng}
      lat={location.position.lat}
      zIndex={focused ? 30 : sos ? 25 : location.online ? 12 : 10}
      offset={offset}
      glide={moving ? "steady" : "settle"}
      // A stand-in for an area has no exact spot to point at: no tail, centred on the area.
      anchor={blurred ? "center" : "bottom"}
    >
      <button
        type="button"
        onClick={onSelect}
        aria-label={name}
        data-testid="friend-pin"
        data-user-id={user.id}
        data-state={state}
        data-precision={blurred ? "blurred" : "precise"}
        className={cn(
          "group flex flex-col items-center outline-none",
          faded && "opacity-70 saturate-50",
        )}
      >
        <span
          className={cn(
            "relative block transition-transform duration-200 ease-bounce",
            focused ? "scale-110" : "group-hover:scale-105 group-focus-visible:scale-105",
            moving && "animate-float",
          )}
        >
          {sos && (
            <span
              className="absolute -inset-2 animate-pulse-ring rounded-[1.6rem] bg-danger"
              aria-hidden
            />
          )}
          {!sos && location.online && !faded && (
            <span
              className="absolute inset-0 animate-pulse-ring rounded-[1.25rem] bg-lime"
              aria-hidden
            />
          )}
          <span
            className={cn(
              "relative block rounded-[1.25rem] bg-surface p-[3px]",
              blurred ? "border-[2.5px] border-dashed border-ink opacity-90" : "sticker",
              focused && "ring-4 ring-brand/50",
              sos && "ring-4 ring-danger",
              "group-focus-visible:ring-4 group-focus-visible:ring-brand",
            )}
          >
            <Avatar
              user={user}
              size="lg"
              className={cn("block [&>span]:rounded-[1rem]", blurred && "blur-[1.5px]")}
            />
          </span>
          {sos ? (
            <span className="sticker-sm absolute -top-3 -left-3 grid h-7 place-items-center rounded-full bg-danger px-1.5 text-[10px] font-black text-white">
              <Siren className="size-3.5" aria-hidden />
              <span className="sr-only">SOS</span>
            </span>
          ) : location.sleeping ? (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-brand text-on-brand">
              <Moon className="size-3.5" />
            </span>
          ) : location.tripId ? (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-sky text-[#17122b]">
              <Navigation className="size-3.5" />
            </span>
          ) : PlaceIcon ? (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-sun text-[#17122b]">
              <PlaceIcon className="size-3.5" />
            </span>
          ) : location.placeEmoji || location.status?.emoji ? (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-surface text-sm">
              {location.placeEmoji || location.status?.emoji}
            </span>
          ) : null}
          {location.batteryPercent !== undefined && (
            <BatteryBadge
              percent={location.batteryPercent}
              charging={location.charging}
              className="absolute -top-3 -right-4"
            />
          )}
        </span>
        {blurred ? (
          <span className="h-1.5" aria-hidden />
        ) : (
          <span className="h-2.5 w-[3px] bg-ink" data-testid="pin-stem" aria-hidden />
        )}
        <span
          data-testid="pin-caption"
          className={cn(
            "rounded-full px-2.5 py-0.5 text-[11px] font-black whitespace-nowrap",
            caption.tone === "blurred"
              ? "border-2 border-dashed border-ink bg-surface text-ink"
              : caption.tone === "calm"
                ? "sticker-sm bg-sun text-[#17122b]"
                : "bg-ink text-surface",
          )}
        >
          {caption.text}
        </span>
        {(focused || showName) && (
          <span className="mt-1 max-w-36 truncate rounded-full bg-surface/90 px-2 py-0.5 text-[11px] font-bold text-ink">
            {name}
          </span>
        )}
      </button>
    </MapPin>
  );
}

/** Pins that overlap at this zoom: one stack of faces with a count. Tapping fans them out. */
export function ClusterMarker({
  lng,
  lat,
  members,
  expanded,
  onSelect,
}: {
  lng: number;
  lat: number;
  members: common.UserSummary[];
  expanded: boolean;
  onSelect(): void;
}) {
  const { t } = useTranslation("map");
  const faces = members.slice(0, 3);
  return (
    <MapPin lng={lng} lat={lat} zIndex={expanded ? 5 : 14} anchor="center">
      <button
        type="button"
        onClick={onSelect}
        aria-expanded={expanded}
        aria-label={t("cluster.label", { count: members.length })}
        data-testid="cluster-pin"
        className={cn(
          "group relative grid place-items-center outline-none transition-all duration-200",
          expanded && "scale-75 opacity-60",
        )}
      >
        <span className="relative flex pr-2">
          {faces.map((user, index) => (
            <span
              key={user.id}
              className={cn(
                "sticker-sm block rounded-[0.95rem] bg-surface p-[2px] group-focus-visible:ring-4 group-focus-visible:ring-brand",
                index > 0 && "-ml-6",
              )}
              style={{ zIndex: 3 - index, transform: `rotate(${(index - 1) * 7}deg)` }}
            >
              <Avatar user={user} size="md" className="block" />
            </span>
          ))}
          <span className="sticker-sm absolute -top-2 -right-1 z-10 grid h-6 min-w-6 place-items-center rounded-full bg-brand px-1.5 text-xs font-black text-on-brand">
            {members.length}
          </span>
        </span>
      </button>
    </MapPin>
  );
}

/** Friends in the same spot: one pin that flips through their faces. */
export function GatheringMarker({
  lng,
  lat,
  members,
  party,
  onSelect,
}: {
  lng: number;
  lat: number;
  members: common.UserSummary[];
  party: boolean;
  onSelect(): void;
}) {
  const { t } = useTranslation("map");
  const [index, setIndex] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setIndex((i) => i + 1), 1200);
    return () => clearInterval(timer);
  }, []);
  const current = members[index % members.length];
  if (!current) return null;
  return (
    <MapPin lng={lng} lat={lat} zIndex={20}>
      <button
        type="button"
        onClick={onSelect}
        aria-label={t("gathering.label", {
          count: members.length,
          names: members.map((member) => member.displayName).join("、"),
        })}
        data-testid="gathering-pin"
        className="group flex flex-col items-center outline-none"
      >
        <span className="relative block transition-transform duration-200 ease-bounce group-hover:scale-105 group-focus-visible:scale-105">
          <span
            className="sticker absolute inset-0 translate-x-2 -translate-y-2 rotate-6 rounded-[1.25rem] bg-pink"
            aria-hidden
          />
          <span
            className="sticker absolute inset-0 -translate-x-2 -translate-y-1 -rotate-6 rounded-[1.25rem] bg-sky"
            aria-hidden
          />
          <span className="sticker relative block rounded-[1.25rem] bg-surface p-[3px] group-focus-visible:ring-4 group-focus-visible:ring-brand">
            <Avatar
              key={current.id}
              user={current}
              size="lg"
              className="block animate-pop-in [&>span]:rounded-[1rem]"
            />
          </span>
          <span
            className="sticker-sm absolute -top-3 -right-3 grid h-7 min-w-7 place-items-center rounded-full bg-coral px-1.5 text-xs font-black text-white"
            aria-hidden
          >
            {party ? "🔥" : members.length}
          </span>
        </span>
        <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
        <span className="sticker-sm rounded-full bg-sun px-2.5 py-0.5 text-[11px] font-black whitespace-nowrap text-[#17122b]">
          {t("gathering.count", { count: members.length })}
        </span>
      </button>
    </MapPin>
  );
}

/** Where a trip is going. */
export function DestinationMarker({
  lng,
  lat,
  label,
}: {
  lng: number;
  lat: number;
  label: string;
}) {
  return (
    <MapPin lng={lng} lat={lat} zIndex={6}>
      <div className="flex flex-col items-center" data-testid="trip-destination">
        <span className="sticker grid size-10 place-items-center rounded-2xl bg-lime text-xl">
          🏁
        </span>
        <span className="h-2 w-[3px] bg-ink" aria-hidden />
        <span className="max-w-40 truncate rounded-full bg-ink px-2.5 py-0.5 text-[11px] font-black text-surface">
          {label}
        </span>
      </div>
    </MapPin>
  );
}

/**
 * The areas of blurred pins: a dashed circle as wide as the uncertainty, so
 * nobody reads a stand-in point as "they are exactly here".
 */
export function ApproximateAreas({
  areas,
}: {
  areas: { id: string; lat: number; lng: number; radiusMeters: number }[];
}) {
  const data: MapGeoJson = {
    type: "FeatureCollection",
    features: areas.map((area) => ({
      type: "Feature",
      properties: { id: area.id },
      geometry: { type: "Polygon", coordinates: [circleRing(area, area.radiusMeters)] },
    })),
  };
  useMapLayer("tokuly-areas", data, [
    {
      id: "tokuly-areas-fill",
      type: "fill",
      paint: { "fill-color": "#7c5cff", "fill-opacity": 0.12 },
    },
    {
      id: "tokuly-areas-line",
      type: "line",
      paint: {
        "line-color": "#7c5cff",
        "line-width": 2,
        "line-dasharray": [2, 2],
        "line-opacity": 0.8,
      },
    },
  ]);
  return null;
}
