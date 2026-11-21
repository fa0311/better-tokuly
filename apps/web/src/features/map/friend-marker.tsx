import { type common, geo, toMillis } from "@tokuly/api";
import { BatteryCharging, BatteryLow, Briefcase, GraduationCap, House, Moon } from "lucide-react";
import { Marker } from "maplibre-gl";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { compactAge, compactDuration, kmh } from "#/lib/format";
import { useMap } from "./map-view";

/** Positions older than this are shown as stale (dimmed, with their age). */
const STALE_AFTER_MS = 10 * 60_000;
/** Faster than this counts as moving (shows the speed badge). */
const MOVING_SPEED_MPS = 0.8;
/** How long a pin takes to glide to a new position. */
const GLIDE_MS = 900;

/**
 * Keeps a MapLibre marker at a coordinate, gliding there when it changes, and
 * renders React children into it.
 */
export function MapPin({
  lng,
  lat,
  zIndex,
  children,
}: {
  lng: number;
  lat: number;
  zIndex?: number;
  children: ReactNode;
}) {
  const map = useMap();
  const [element] = useState(() => document.createElement("div"));
  const marker = useRef<Marker | null>(null);
  const shown = useRef<[number, number] | null>(null);

  useEffect(() => {
    const instance = new Marker({ element, anchor: "bottom" });
    marker.current = instance;
    return () => {
      instance.remove();
      marker.current = null;
      shown.current = null;
    };
  }, [element]);

  useEffect(() => {
    const instance = marker.current;
    if (!instance) return;
    const from = shown.current;
    if (!from) {
      // First position: appear in place.
      instance.setLngLat([lng, lat]).addTo(map);
      shown.current = [lng, lat];
      return;
    }
    const start = performance.now();
    let frame = 0;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / GLIDE_MS);
      const eased = 1 - (1 - t) ** 3;
      const position: [number, number] = [from[0] + (lng - from[0]) * eased, from[1] + (lat - from[1]) * eased];
      instance.setLngLat(position);
      shown.current = position;
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [map, lng, lat]);

  useEffect(() => {
    element.style.zIndex = String(zIndex ?? 1);
  }, [element, zIndex]);

  return createPortal(children, element);
}

const PLACE_ICONS = {
  [geo.PlaceLabel.HOME]: House,
  [geo.PlaceLabel.WORK]: Briefcase,
  [geo.PlaceLabel.SCHOOL]: GraduationCap,
} as const;

/** Re-renders on an interval so "NOW" / "12分" stay current. */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

/** The text under a pin: speed while moving, time on the spot, or age when stale. */
function pinCaption(location: geo.FriendLocation, now: number): { text: string; tone: "live" | "calm" | "stale" } {
  const recordedAt = toMillis(location.recordedAt);
  if (now - recordedAt > STALE_AFTER_MS) return { text: compactAge(recordedAt, now), tone: "stale" };
  if (location.speedMps >= MOVING_SPEED_MPS) return { text: `${kmh(location.speedMps)} KM/H`, tone: "live" };
  const since = toMillis(location.stationarySince);
  if (since > 0 && now - since >= 5 * 60_000) return { text: compactDuration(now - since), tone: "calm" };
  return { text: compactAge(recordedAt, now), tone: "live" };
}

export interface FriendMarkerProps {
  user: common.UserSummary;
  location: geo.FriendLocation;
  focused: boolean;
  onSelect(): void;
}

/**
 * A friend on the map: their face as a sticker with a tail, plus the little
 * signals around it (battery, place, sleep, speed, how long they have been
 * there).
 */
export function FriendMarker({ user, location, focused, onSelect }: FriendMarkerProps) {
  const now = useNow(10_000);
  if (!location.position) return null;
  const caption = pinCaption(location, now);
  const stale = caption.tone === "stale";
  const moving = !stale && location.speedMps >= MOVING_SPEED_MPS;
  const approximate = location.precision === geo.GhostMode.BLURRED;
  const PlaceIcon = PLACE_ICONS[location.placeLabel as keyof typeof PLACE_ICONS];
  const lowBattery = location.batteryPercent !== undefined && location.batteryPercent <= 20 && !location.charging;

  return (
    <MapPin lng={location.position.lng} lat={location.position.lat} zIndex={focused ? 30 : location.online ? 12 : 10}>
      <button
        type="button"
        onClick={onSelect}
        aria-label={user.displayName}
        className={cn("group flex flex-col items-center outline-none", stale && "opacity-70 saturate-50")}
      >
        <span className={cn("relative block transition-transform duration-200 ease-bounce", focused ? "scale-110" : "group-hover:scale-105", moving && "animate-float")}>
          {location.online && !stale && <span className="absolute inset-0 animate-pulse-ring rounded-[1.25rem] bg-lime" aria-hidden />}
          <span
            className={cn(
              "relative block rounded-[1.25rem] bg-surface p-[3px]",
              approximate ? "border-[2.5px] border-dashed border-ink" : "sticker",
              focused && "ring-4 ring-brand/50",
            )}
          >
            <Avatar user={user} size="lg" className="block [&>span]:rounded-[1rem]" />
          </span>
          {location.sleeping && (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-brand text-on-brand">
              <Moon className="size-3.5" />
            </span>
          )}
          {!location.sleeping && PlaceIcon && (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-sun text-[#17122b]">
              <PlaceIcon className="size-3.5" />
            </span>
          )}
          {!location.sleeping && !PlaceIcon && location.status?.emoji && (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-surface text-sm">{location.status.emoji}</span>
          )}
          {(lowBattery || location.charging) && (
            <span
              className={cn(
                "sticker-sm absolute -top-3 -right-3 flex h-7 items-center gap-0.5 rounded-full px-1.5 text-[10px] font-black",
                lowBattery ? "bg-danger text-white" : "bg-mint text-[#17122b]",
              )}
            >
              {location.charging ? <BatteryCharging className="size-3.5" /> : <BatteryLow className="size-3.5" />}
              {location.batteryPercent !== undefined && `${location.batteryPercent}`}
            </span>
          )}
        </span>
        <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
        <span
          className={cn(
            "rounded-full px-2.5 py-0.5 text-[11px] font-black whitespace-nowrap",
            caption.tone === "live" && caption.text === "NOW" ? "bg-lime text-[#17122b] sticker-sm" : "bg-ink text-surface",
          )}
        >
          {approximate ? `〜 ${caption.text}` : caption.text}
        </span>
        {focused && <span className="mt-1 max-w-36 truncate rounded-full bg-surface/90 px-2 py-0.5 text-[11px] font-bold text-ink">{user.displayName}</span>}
      </button>
    </MapPin>
  );
}

export interface GatheringMarkerProps {
  lng: number;
  lat: number;
  members: common.UserSummary[];
  onSelect(): void;
}

/** Friends in the same spot: one pin that flips through their faces. */
export function GatheringMarker({ lng, lat, members, onSelect }: GatheringMarkerProps) {
  const [index, setIndex] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setIndex((i) => i + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const current = members[index % members.length];
  if (!current) return null;
  return (
    <MapPin lng={lng} lat={lat} zIndex={20}>
      <button type="button" onClick={onSelect} className="group flex flex-col items-center outline-none">
        <span className="relative block transition-transform duration-200 ease-bounce group-hover:scale-105">
          <span className="sticker absolute inset-0 translate-x-2 -translate-y-2 rotate-6 rounded-[1.25rem] bg-pink" aria-hidden />
          <span className="sticker absolute inset-0 -translate-x-2 -translate-y-1 -rotate-6 rounded-[1.25rem] bg-sky" aria-hidden />
          <span className="sticker relative block rounded-[1.25rem] bg-surface p-[3px]">
            <Avatar key={current.id} user={current} size="lg" className="block animate-pop-in [&>span]:rounded-[1rem]" />
          </span>
          <span className="sticker-sm absolute -top-3 -right-3 grid h-7 min-w-7 place-items-center rounded-full bg-coral px-1.5 text-xs font-black text-white">
            {members.length}
          </span>
        </span>
        <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
        <span className="sticker-sm rounded-full bg-sun px-2.5 py-0.5 text-[11px] font-black whitespace-nowrap text-[#17122b]">{members.length}人が集合中!</span>
      </button>
    </MapPin>
  );
}
