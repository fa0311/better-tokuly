import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, geo, social, toMillis } from "@tokuly/api";
import { Bell, Eye, GlassWater, Hand, Heart, LocateFixed, MessageCircle, Navigation, UserRound, Users, X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { api } from "#/lib/api";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge, compactDuration, distanceMeters, formatDistance, kmh } from "#/lib/format";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { useFriendLocationsSync, useFriendsById } from "./data";
import { FriendMarker, GatheringMarker, MapPin } from "./friend-marker";
import { groupByProximity } from "./grouping";
import { MapView, useMap } from "./map-view";
import { useOwnLocation } from "./own-location";

/** Friends this close together are shown as one gathering. */
const GATHERING_RADIUS_M = 30;
/** Positions older than this do not count towards a gathering. */
const GATHERING_FRESHNESS_MS = 15 * 60_000;
const LAST_VIEW_KEY = "tokuly.map.view";
const TOKYO: [number, number] = [139.767125, 35.681236];

interface SavedView {
  center: [number, number];
  zoom: number;
  /** False on a first visit, when the view is only a placeholder until we know where the user is. */
  remembered: boolean;
}

function loadSavedView(): SavedView {
  try {
    const saved = JSON.parse(window.localStorage.getItem(LAST_VIEW_KEY) ?? "null") as SavedView | null;
    if (saved && Array.isArray(saved.center) && typeof saved.zoom === "number") return { ...saved, remembered: true };
  } catch {
    // Fall through to the default view.
  }
  return { center: TOKYO, zoom: 11, remembered: false };
}

/** The home screen: the live friend map with its floating chrome. */
export function MapScreen() {
  useFriendLocationsSync();
  useOwnLocation(true);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [view] = useState(loadSavedView);

  return (
    <MapView styleId="tokuly" initialCenter={view.center} initialZoom={view.zoom}>
      <RememberView />
      {!view.remembered && <CenterOnFirstFix />}
      <FriendLayer focusedId={focusedId} onFocus={setFocusedId} />
      <OwnMarker />
      <MapTitle />
      <LocationNotice />
      <FriendCarousel focusedId={focusedId} onFocus={setFocusedId} />
      <MapButtons />
      {focusedId && <FriendCard userId={focusedId} onClose={() => setFocusedId(null)} />}
    </MapView>
  );
}

/** Reopens the map where the user left it. */
function RememberView() {
  const map = useMap();
  useEffect(() => {
    const save = () => {
      const center = map.getCenter();
      window.localStorage.setItem(LAST_VIEW_KEY, JSON.stringify({ center: [center.lng, center.lat], zoom: map.getZoom() }));
    };
    map.on("moveend", save);
    return () => {
      map.off("moveend", save);
    };
  }, [map]);
  return null;
}

/** On a first visit there is no remembered view: go to the user as soon as their position is known. */
function CenterOnFirstFix() {
  const map = useMap();
  const own = useLocations((s) => s.own);
  const centered = useRef(false);
  useEffect(() => {
    if (!own || centered.current) return;
    centered.current = true;
    map.easeTo({ center: [own.lng, own.lat], zoom: 15, duration: 800 * dopamineFactor(), essential: true });
  }, [map, own]);
  return null;
}

function useFlyTo() {
  const map = useMap();
  return (lng: number, lat: number, minZoom = 15) =>
    map.flyTo({ center: [lng, lat], zoom: Math.max(map.getZoom(), minZoom), speed: 1.6, curve: 1.4, essential: true });
}

function FriendLayer({ focusedId, onFocus }: { focusedId: string | null; onFocus(id: string | null): void }) {
  const friends = useFriendsById();
  const locations = useLocations((s) => s.friends);
  const flyTo = useFlyTo();
  const now = Date.now();

  const visible = Object.values(locations).filter((l) => l.position && friends[l.userId]?.user);
  const fresh = visible.filter((l) => now - toMillis(l.recordedAt) <= GATHERING_FRESHNESS_MS);
  const groups = groupByProximity(
    fresh.map((l) => ({ id: l.userId, lat: l.position!.lat, lng: l.position!.lng })),
    GATHERING_RADIUS_M,
  ).filter((group) => group.members.length >= 2);
  const gathered = new Set(groups.flatMap((group) => group.members.map((m) => m.id)));

  return (
    <>
      {visible
        .filter((l) => !gathered.has(l.userId))
        .map((l) => (
          <FriendMarker
            key={l.userId}
            user={friends[l.userId]!.user!}
            location={l}
            focused={focusedId === l.userId}
            onSelect={() => {
              onFocus(l.userId);
              flyTo(l.position!.lng, l.position!.lat);
            }}
          />
        ))}
      {groups.map((group) => (
        <GatheringMarker
          key={group.members
            .map((m) => m.id)
            .sort()
            .join("+")}
          lng={group.lng}
          lat={group.lat}
          members={group.members.map((m) => friends[m.id]!.user!)}
          onSelect={() => {
            onFocus(group.members[0]!.id);
            flyTo(group.lng, group.lat, 17);
          }}
        />
      ))}
    </>
  );
}

function OwnMarker() {
  const { t } = useTranslation("map");
  const me = useMe();
  const own = useLocations((s) => s.own);
  if (!own || !me?.user) return null;
  return (
    <MapPin lng={own.lng} lat={own.lat} zIndex={8}>
      <div className="flex flex-col items-center">
        <span className="relative block">
          <span className="absolute inset-0 animate-pulse-ring rounded-[1.25rem] bg-brand" aria-hidden />
          <span className="sticker relative block rounded-[1.25rem] bg-brand p-[3px]">
            <Avatar user={me.user} size="lg" className="block [&>span]:rounded-[1rem]" />
          </span>
        </span>
        <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
        <span className="rounded-full bg-brand px-2.5 py-0.5 text-[11px] font-black text-on-brand">{t("you")}</span>
      </div>
    </MapPin>
  );
}

/** Big place name of whatever the map is centred on; "World" when zoomed out. */
function MapTitle() {
  const { t, i18n } = useTranslation("map");
  const map = useMap();
  const [view, setView] = useState(() => ({ lat: map.getCenter().lat, lng: map.getCenter().lng, zoom: map.getZoom() }));
  useEffect(() => {
    const update = () => setView({ lat: map.getCenter().lat, lng: map.getCenter().lng, zoom: map.getZoom() });
    map.on("moveend", update);
    return () => {
      map.off("moveend", update);
    };
  }, [map]);

  const zoomedOut = view.zoom < 4.5;
  // Round so that small pans reuse the cached name.
  const precision = view.zoom >= 12 ? 3 : view.zoom >= 8 ? 2 : 1;
  const lat = Number(view.lat.toFixed(precision));
  const lng = Number(view.lng.toFixed(precision));
  const { data } = useQuery({
    queryKey: ["geo", "reverse", lat, lng, i18n.language],
    queryFn: () => api().service(geo.PlaceService).reverseGeocode({ position: { lat, lng }, language: i18n.language }),
    enabled: !zoomedOut,
    staleTime: 10 * 60_000,
    retry: false,
  });
  const name = zoomedOut ? t("world") : (data?.result?.name ?? "");
  if (!name) return null;
  return (
    <div className="pointer-events-none absolute top-0 left-0 z-20 p-4 pt-safe md:p-6">
      <h1 key={name} className="animate-slide-up text-4xl font-black tracking-tight text-ink drop-shadow-[0_2px_0_var(--surface)] md:text-5xl">
        {name}
      </h1>
      {!zoomedOut && data?.result?.region && <p className="mt-1 text-sm font-bold text-ink-soft drop-shadow-[0_1px_0_var(--surface)]">{data.result.region}</p>}
    </div>
  );
}

function LocationNotice() {
  const { t } = useTranslation("map");
  const error = useLocations((s) => s.ownError);
  if (!error) return null;
  return (
    <div className="absolute inset-x-0 top-0 z-20 flex justify-center p-4 pt-safe">
      <p className="glass sticker-sm max-w-md rounded-2xl px-4 py-3 text-sm font-bold">{error === "denied" ? t("locationDenied") : t("locationUnavailable")}</p>
    </div>
  );
}

/** The row of friends along the bottom: tap a face to fly to them. */
function FriendCarousel({ focusedId, onFocus }: { focusedId: string | null; onFocus(id: string): void }) {
  const { t } = useTranslation("map");
  const friends = useFriendsById();
  const locations = useLocations((s) => s.friends);
  const online = useLocations((s) => s.online);
  const own = useLocations((s) => s.own);
  const flyTo = useFlyTo();

  const entries = Object.values(locations)
    .filter((l) => l.position && friends[l.userId]?.user)
    .map((l) => ({ location: l, user: friends[l.userId]!.user!, distance: own ? distanceMeters(own, l.position!) : Number.POSITIVE_INFINITY }))
    .sort((a, b) => a.distance - b.distance);

  if (focusedId) return null;
  if (entries.length === 0) {
    return (
      <div className="absolute inset-x-0 bottom-24 z-20 flex justify-center px-4 md:bottom-8">
        <div className="glass sticker w-full max-w-sm rounded-3xl p-4">
          <div className="flex items-center gap-3">
            <div className="text-4xl">🫡</div>
            <div className="min-w-0 flex-1">
              <p className="font-black">{t("noFriendsTitle")}</p>
              <p className="text-sm text-ink-soft">{t("noFriendsBody")}</p>
            </div>
          </div>
          <Link to="/friends" className="pressable sticker-sm mt-3 flex h-11 w-full items-center justify-center rounded-full bg-lime text-sm font-black text-[#17122b]">
            {t("addFriends")}
          </Link>
        </div>
      </div>
    );
  }
  return (
    <div className="absolute inset-x-0 bottom-24 z-20 flex justify-center md:bottom-6">
      <div className="scrollbar-none flex max-w-full gap-3 overflow-x-auto px-4 py-2">
        {entries.map(({ location, user, distance }) => (
          <button
            key={user.id}
            type="button"
            onClick={() => {
              onFocus(user.id);
              flyTo(location.position!.lng, location.position!.lat);
            }}
            className="pressable flex w-16 shrink-0 flex-col items-center gap-1"
          >
            <Avatar user={user} size="lg" outlined online={online[user.id]} />
            <span className="glass w-full truncate rounded-full px-1.5 text-center text-[10px] font-black">
              {Number.isFinite(distance) ? formatDistance(distance) : user.displayName}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function MapButtons() {
  const { t } = useTranslation("map");
  const map = useMap();
  const own = useLocations((s) => s.own);
  const locations = useLocations((s) => s.friends);
  const flyTo = useFlyTo();

  const showEveryone = () => {
    const points = Object.values(locations)
      .filter((l) => l.position)
      .map((l) => [l.position!.lng, l.position!.lat] as [number, number]);
    if (own) points.push([own.lng, own.lat]);
    if (points.length === 0) return;
    const lngs = points.map((p) => p[0]);
    const lats = points.map((p) => p[1]);
    map.fitBounds(
      [
        [Math.min(...lngs), Math.min(...lats)],
        [Math.max(...lngs), Math.max(...lats)],
      ],
      { padding: 96, maxZoom: 15, duration: 900 },
    );
  };

  return (
    <div className="absolute top-0 right-0 z-20 flex flex-col gap-2 p-4 pt-safe md:p-6">
      <RoundButton label={t("recenter")} disabled={!own} onClick={() => own && flyTo(own.lng, own.lat, 16)}>
        <LocateFixed className="size-5" />
      </RoundButton>
      <RoundButton label={t("showEveryone")} onClick={showEveryone}>
        <Users className="size-5" />
      </RoundButton>
    </div>
  );
}

function RoundButton({ label, children, ...props }: { label: string; children: ReactNode; disabled?: boolean; onClick(): void }) {
  return (
    <button type="button" aria-label={label} title={label} className="glass sticker-sm pressable grid size-12 place-items-center rounded-full disabled:opacity-50" {...props}>
      {children}
    </button>
  );
}

/** The panel for a selected friend: who, where, how they are doing, and things to send them. */
function FriendCard({ userId, onClose }: { userId: string; onClose(): void }) {
  const { t } = useTranslation("map");
  const friend = useFriendsById()[userId];
  const location = useLocations((s) => s.friends[userId]);
  const own = useLocations((s) => s.own);
  const user = friend?.user;
  if (!user) return null;

  const now = Date.now();
  const facts: string[] = [];
  if (location) {
    const moving = location.speedMps >= 0.8;
    const since = toMillis(location.stationarySince);
    if (location.sleeping) facts.push(t("sleeping"));
    if (moving) facts.push(t("moving", { speed: kmh(location.speedMps) }));
    else if (since > 0) facts.push(t("stationaryFor", { duration: compactDuration(now - since) }));
    if (location.batteryPercent !== undefined) {
      facts.push(location.charging ? `${t("battery", { percent: location.batteryPercent })} · ${t("charging")}` : t("battery", { percent: location.batteryPercent }));
    }
    if (location.precision === geo.GhostMode.BLURRED) facts.push(t("approximate"));
    facts.push(t("lastSeen", { age: compactAge(toMillis(location.recordedAt), now) }));
  }
  const distance = own && location?.position ? distanceMeters(own, location.position) : null;
  const placeName =
    location?.placeLabel === geo.PlaceLabel.HOME
      ? t("place.home")
      : location?.placeLabel === geo.PlaceLabel.WORK
        ? t("place.work")
        : location?.placeLabel === geo.PlaceLabel.SCHOOL
          ? t("place.school")
          : location?.placeName || location?.locality;

  return (
    <aside className="absolute inset-x-0 bottom-0 z-30 p-3 pb-24 md:inset-x-auto md:right-6 md:bottom-6 md:w-[380px] md:p-0">
      <div className="glass sticker-lg animate-slide-up rounded-[2rem] p-5">
        <div className="flex items-start gap-4">
          <Avatar user={user} size="xl" outlined online={location?.online} />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-xl font-black">{friend.nickname || user.displayName}</h2>
            <p className="truncate text-sm font-bold text-ink-soft">@{user.handle}</p>
            {(location?.status?.emoji || location?.status?.text) && (
              <p className="mt-1 truncate text-sm">
                {location.status.emoji} {location.status.text}
              </p>
            )}
          </div>
          <button type="button" onClick={onClose} aria-label="close" className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2">
            <X className="size-5" />
          </button>
        </div>

        {(placeName || distance !== null) && (
          <p className="mt-4 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold">
            {placeName && <span className="mr-2">📍 {placeName}</span>}
            {distance !== null && <span className="text-ink-soft">{t("distanceAway", { distance: formatDistance(distance) })}</span>}
          </p>
        )}
        {facts.length > 0 && (
          <ul className="mt-3 flex flex-wrap gap-1.5">
            {facts.map((fact) => (
              <li key={fact} className="rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
                {fact}
              </li>
            ))}
          </ul>
        )}

        <ReactionBar user={user} />

        <div className="mt-4 grid grid-cols-3 gap-2">
          <Link to="/chat" search={{ user: user.id }} className="pressable sticker-sm flex h-11 items-center justify-center gap-1.5 rounded-full bg-brand text-sm font-black text-on-brand">
            <MessageCircle className="size-4" />
            {t("actions.chat")}
          </Link>
          <Link to="/u/$handle" params={{ handle: user.handle }} className="pressable sticker-sm flex h-11 items-center justify-center gap-1.5 rounded-full bg-surface text-sm font-black">
            <UserRound className="size-4" />
            {t("actions.profile")}
          </Link>
          <a
            href={location?.position ? `https://www.google.com/maps/dir/?api=1&destination=${location.position.lat},${location.position.lng}` : undefined}
            target="_blank"
            rel="noreferrer"
            aria-disabled={!location?.position}
            className={cn("pressable sticker-sm flex h-11 items-center justify-center gap-1.5 rounded-full bg-surface text-sm font-black", !location?.position && "pointer-events-none opacity-50")}
          >
            <Navigation className="size-4" />
            {t("actions.directions")}
          </a>
        </div>
      </div>
    </aside>
  );
}

/** Things to throw at a friend's pin: hearts, eyes, drinks, a knock, a buzz. */
function ReactionBar({ user }: { user: common.UserSummary }) {
  const { t } = useTranslation("map");
  const [busy, setBusy] = useState<string | null>(null);
  const interactions = () => api().service(social.InteractionService);

  const run = async (key: string, action: () => Promise<unknown>, done: string) => {
    setBusy(key);
    try {
      await action();
      toast.success(done);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setBusy(null);
    }
  };
  const react = (key: string, kind: social.MapReactionKind) =>
    run(key, () => interactions().sendMapReaction({ userIds: [user.id], kind, intensity: 1 }), t("sent.reaction"));

  return (
    <div className="mt-4 flex justify-between gap-1">
      <Button variant="ghost" size="icon" aria-label={t("actions.hearts")} title={t("actions.hearts")} loading={busy === "hearts"} onClick={() => react("hearts", social.MapReactionKind.HEARTS)}>
        <Heart className="size-6 text-pink" />
      </Button>
      <Button variant="ghost" size="icon" aria-label={t("actions.eyes")} title={t("actions.eyes")} loading={busy === "eyes"} onClick={() => react("eyes", social.MapReactionKind.EYES)}>
        <Eye className="size-6 text-sky" />
      </Button>
      <Button variant="ghost" size="icon" aria-label={t("actions.drinks")} title={t("actions.drinks")} loading={busy === "drinks"} onClick={() => react("drinks", social.MapReactionKind.DRINKS)}>
        <GlassWater className="size-6 text-coral" />
      </Button>
      <Button variant="ghost" size="icon" aria-label={t("actions.knock")} title={t("actions.knock")} loading={busy === "knock"} onClick={() => run("knock", () => interactions().knock({ userId: user.id, count: 1 }), t("sent.knock"))}>
        <Hand className="size-6 text-sun" />
      </Button>
      <Button variant="ghost" size="icon" aria-label={t("actions.buzz")} title={t("actions.buzz")} loading={busy === "buzz"} onClick={() => run("buzz", () => interactions().buzz({ userId: user.id }), t("sent.buzz"))}>
        <Bell className="size-6 text-brand" />
      </Button>
    </div>
  );
}
