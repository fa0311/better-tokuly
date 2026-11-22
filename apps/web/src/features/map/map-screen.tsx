import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { common, geo } from "@tokuly/api";
import {
  BellRing,
  Compass,
  Ghost,
  Link2,
  LocateFixed,
  Minus,
  MoreHorizontal,
  Navigation,
  Plus,
  UserRound,
  Users,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Menu } from "#/components/ui/menu";
import { cn } from "#/lib/cn";
import { formatDistance } from "#/lib/format";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { useShakeToBump } from "./bump";
import { recentAim, useFlyTo, useFlyToUser } from "./camera";
import {
  friendName,
  placeClient,
  useActiveSos,
  useFriendsById,
  useGatherings,
  useGhostExpiryRefetch,
  useGhostSettings,
  useMapSync,
  useNow,
} from "./data";
import { EffectsLayer } from "./effects-layer";
import { FriendListPanel } from "./friend-list";
import { FriendSheet } from "./friend-sheet";
import { GatheringSheet } from "./gathering-sheet";
import { GhostIndicator, GhostPanel } from "./ghost";
import { DEFAULT_CENTER, fitMapToPoints, MapPin, MapView, useMap } from "./map-view";
import { MeSheet } from "./me-sheet";
import {
  clusterOnScreen,
  distanceTo,
  fanOffset,
  ghostRuleFor,
  ghostSummary,
  isBlurred,
  pinState,
  seeEachOther,
} from "./model";
import { useSharing, useSharingState } from "./own-location";
import {
  ApproximateAreas,
  ClusterMarker,
  FriendMarker,
  GatheringMarker,
  useMapValue,
} from "./pins";
import { SafetyPanel, SosBanners } from "./safety";
import { SharesPanel } from "./shares";
import { useCoveredInset, useIsWide } from "./sheet";
import { StoryPanel, StoryTray } from "./stories";
import { TripChips, TripLayer, TripsPanel } from "./trips";
import { useFocusedUserId, useMapUi } from "./ui-store";

const LAST_VIEW_KEY = "tokuly.map.view";
/** Pins closer than this on screen are drawn as one stack. */
const CLUSTER_PIXELS = 46;
/** Pins this far outside the screen are still drawn, so they are there when the map pans. */
const OFFSCREEN_MARGIN = 160;
/** How often, at most, the pins are laid out again while the map moves. */
const LAYOUT_EVERY_MS = 120;
/** A crowd up to this size fans out on tap; a bigger one zooms in instead. */
const FAN_LIMIT = 8;
/** Names appear under pins from this zoom. */
const NAME_ZOOM = 13;
/** Space the phone's floating tab bar takes at the bottom of the map. */
const TAB_BAR = 84;

interface SavedView {
  center: [number, number];
  zoom: number;
  /** False on a first visit, when the view is only a placeholder until we know where the user is. */
  remembered: boolean;
}

function loadSavedView(): SavedView {
  try {
    const saved = JSON.parse(
      window.localStorage.getItem(LAST_VIEW_KEY) ?? "null",
    ) as SavedView | null;
    if (saved && Array.isArray(saved.center) && typeof saved.zoom === "number")
      return { ...saved, remembered: true };
  } catch {
    // Fall through to the default view.
  }
  return { center: DEFAULT_CENTER, zoom: 11, remembered: false };
}

/**
 * The home screen: the live friend map with its floating chrome.
 * `focusUserId` opens that friend's sheet (a link from a notification or a
 * profile: `/map?friend=<user id>`).
 */
export function MapScreen({ focusUserId }: { focusUserId?: string }) {
  useMapSync();
  useGhostExpiryRefetch();
  useShakeToBump();
  const [view] = useState(loadSavedView);

  return (
    <div data-map-frame className="absolute inset-0 overflow-hidden">
      <MapView initialCenter={view.center} initialZoom={view.zoom}>
        <Camera centerOnFirstFix={!view.remembered} />
        <RememberView />
        <FriendLayer />
        <TripLayer />
        <OwnMarker />
        <TopChrome />
        <MapButtons />
        <BottomChrome />
        <PanelHost />
        <EffectsLayer />
        {focusUserId && <FocusFriend userId={focusUserId} />}
      </MapView>
    </div>
  );
}

/**
 * Keeps the camera honest about what is visible: the parts of the map under
 * floating chrome become padding, so "centre on me" lands in the middle of
 * what can be seen. On a first visit it also brings the map to the person as
 * soon as their position is known, until they move the map themselves.
 */
function Camera({ centerOnFirstFix }: { centerOnFirstFix: boolean }) {
  const map = useMap();
  const insets = useMapUi((state) => state.insets);
  const wide = useIsWide();
  const hasOwn = useLocations((state) => state.own !== null);
  const steering = useRef(centerOnFirstFix);
  const first = useRef(true);

  const max = (side: "top" | "bottom" | "left") =>
    Math.max(
      0,
      ...Object.values(insets)
        .filter((inset) => inset.side === side)
        .map((inset) => inset.size),
    );
  const top = max("top");
  const bottom = Math.max(wide ? 0 : TAB_BAR, max("bottom"));
  const left = max("left");

  useEffect(() => {
    const stop = (event: { originalEvent?: unknown }) => {
      if (event.originalEvent) steering.current = false;
    };
    map.on("movestart", stop);
    return () => {
      map.off("movestart", stop);
    };
  }, [map]);

  useEffect(() => {
    // Never squeeze the visible part to nothing on a short screen.
    const room = Math.max(0, map.getContainer().clientHeight - 180);
    const scale = top + bottom > room ? room / (top + bottom) : 1;
    const padding = { top: top * scale, bottom: bottom * scale, left, right: 0 };
    const own = useLocations.getState().own;
    const target = recentAim();
    if (target) {
      // A sheet opened because of a tap that also flew somewhere: land there, in what is still visible.
      steering.current = false;
      map.easeTo({
        padding,
        center: [target.lng, target.lat],
        zoom: target.zoom,
        duration: 600,
        essential: true,
      });
    } else if (steering.current && hasOwn && own) {
      map.easeTo({
        padding,
        center: [own.lng, own.lat],
        zoom: 15,
        duration: first.current ? 0 : 600,
        essential: true,
      });
    } else {
      map.easeTo({ padding, duration: first.current ? 0 : 300, essential: true });
    }
    first.current = false;
  }, [map, top, bottom, left, hasOwn]);

  // Once the first fix has had time to settle, the map is the person's to move.
  useEffect(() => {
    if (!hasOwn) return;
    const timer = setTimeout(() => {
      steering.current = false;
    }, 4000);
    return () => clearTimeout(timer);
  }, [hasOwn]);

  return null;
}

/** Reopens the map where the user left it. */
function RememberView() {
  const map = useMap();
  useEffect(() => {
    const save = () => {
      const center = map.getCenter();
      window.localStorage.setItem(
        LAST_VIEW_KEY,
        JSON.stringify({ center: [center.lng, center.lat], zoom: map.getZoom() }),
      );
    };
    map.on("moveend", save);
    return () => {
      map.off("moveend", save);
    };
  }, [map]);
  return null;
}

/** Opens a friend's sheet once, when the page was asked to (`?friend=`). */
function FocusFriend({ userId }: { userId: string }) {
  const flyToUser = useFlyToUser();
  const known = useLocations((state) => Boolean(state.friends[userId]?.position));
  const done = useRef("");
  useEffect(() => {
    if (done.current !== userId) useMapUi.getState().open({ kind: "friend", userId });
    if (known && done.current !== `${userId}:flown`) {
      flyToUser(userId);
      done.current = `${userId}:flown`;
    } else if (!known) {
      done.current = userId;
    }
  }, [userId, known]);
  return null;
}

/** Every friend's pin: alone, stacked with others that overlap, or as part of a gathering. */
function FriendLayer() {
  const map = useMap();
  const friends = useFriendsById();
  const locations = useLocations((state) => state.friends);
  const { data: gatherings } = useGatherings();
  const { data: sos } = useActiveSos();
  const focusedId = useFocusedUserId();
  const flyTo = useFlyTo();
  const flyToUser = useFlyToUser();
  const showNames = useMapValue((instance) => instance.getZoom() >= NAME_ZOOM, "zoom");
  const [layout, setLayout] = useState<{ clusters: string[][]; onScreen: string[] }>({
    clusters: [],
    onScreen: [],
  });
  const [expanded, setExpanded] = useState<string | null>(null);
  const { open } = useMapUi();

  const gathered = new Set(
    (gatherings ?? []).flatMap((gathering) =>
      gathering.members.map((member) => member.user?.id ?? ""),
    ),
  );
  const inSos = new Set((sos?.friends ?? []).map((entry) => entry.user?.id ?? ""));
  const visible = Object.values(locations).filter(
    (location) =>
      location.position && friends[location.userId]?.user && !gathered.has(location.userId),
  );
  const signature = visible
    .map(
      (location) =>
        `${location.userId}:${location.position?.lat.toFixed(5)},${location.position?.lng.toFixed(5)}`,
    )
    .join("|");

  // Which pins are on screen and which of them overlap depends on the camera: look again
  // when a pin moves and while the map does — a few times a second, not on every frame.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const compute = () => {
      timer = undefined;
      const box = map.getContainer();
      const points: { id: string; x: number; y: number }[] = [];
      const onScreen: string[] = [];
      for (const location of visible) {
        const point = map.project([location.position?.lng ?? 0, location.position?.lat ?? 0]);
        const near =
          point.x > -OFFSCREEN_MARGIN &&
          point.y > -OFFSCREEN_MARGIN &&
          point.x < box.clientWidth + OFFSCREEN_MARGIN &&
          point.y < box.clientHeight + OFFSCREEN_MARGIN;
        // The friend whose sheet is open always keeps a pin of their own.
        if (location.userId === focusedId) onScreen.push(location.userId);
        else if (near) {
          onScreen.push(location.userId);
          points.push({ id: location.userId, x: point.x, y: point.y });
        }
      }
      const next = { clusters: clusterOnScreen(points, CLUSTER_PIXELS), onScreen: onScreen.sort() };
      setLayout((current) => (JSON.stringify(current) === JSON.stringify(next) ? current : next));
    };
    const schedule = () => {
      timer ??= setTimeout(compute, LAYOUT_EVERY_MS);
    };
    compute();
    map.on("move", schedule);
    map.on("moveend", schedule);
    return () => {
      clearTimeout(timer);
      map.off("move", schedule);
      map.off("moveend", schedule);
    };
  }, [map, signature, focusedId]);
  const { clusters } = layout;
  const onScreen = new Set(layout.onScreen);

  // A fanned-out crowd folds back when the map itself is tapped.
  useEffect(() => {
    const collapse = (event: { originalEvent: MouseEvent }) => {
      if (event.originalEvent.target === map.getCanvas()) setExpanded(null);
    };
    map.on("click", collapse);
    return () => {
      map.off("click", collapse);
    };
  }, [map]);

  const select = (userId: string) => {
    open({ kind: "friend", userId });
    flyToUser(userId);
  };

  const clusterOf = new Map<string, string>();
  for (const ids of clusters) for (const id of ids) clusterOf.set(id, ids.join("+"));
  const marker = (location: geo.FriendLocation, offset?: [number, number]) => {
    const friend = friends[location.userId];
    const user = friend?.user;
    if (!user) return null;
    return (
      <FriendMarker
        key={location.userId}
        user={user}
        name={friendName(friend, user)}
        location={location}
        focused={focusedId === location.userId}
        sos={inSos.has(location.userId)}
        showName={showNames || offset !== undefined}
        offset={offset}
        onSelect={() => select(location.userId)}
      />
    );
  };

  const areas = Object.values(locations)
    .filter(
      (location) =>
        location.position &&
        friends[location.userId]?.user &&
        // Whatever its age: a stand-in point must never be read as a position.
        isBlurred(location),
    )
    .map((location) => ({
      id: location.userId,
      lat: location.position?.lat ?? 0,
      lng: location.position?.lng ?? 0,
      radiusMeters: Math.max(500, location.accuracyMeters),
    }));

  return (
    <>
      <ApproximateAreas areas={areas} />
      {/* Only pins on (or just off) the screen exist as elements: hundreds of friends stay cheap. */}
      {visible
        .filter((location) => onScreen.has(location.userId) && !clusterOf.has(location.userId))
        .map((location) => marker(location))}
      {clusters.map((ids) => {
        const key = ids.join("+");
        const members = ids
          .map((id) => locations[id])
          .filter((location): location is geo.FriendLocation =>
            Boolean(location?.position && friends[location.userId]?.user),
          );
        if (members.length < 2) return members.map((location) => marker(location));
        const lat =
          members.reduce((sum, location) => sum + (location.position?.lat ?? 0), 0) /
          members.length;
        const lng =
          members.reduce((sum, location) => sum + (location.position?.lng ?? 0), 0) /
          members.length;
        const isOpen = expanded === key;
        return (
          <ClusterGroup key={key}>
            <ClusterMarker
              lat={lat}
              lng={lng}
              members={members.map(
                (location) => friends[location.userId]?.user as common.UserSummary,
              )}
              expanded={isOpen}
              onSelect={() => {
                if (isOpen) setExpanded(null);
                else if (members.length <= FAN_LIMIT) setExpanded(key);
                else
                  fitMapToPoints(
                    map,
                    members.map((location) => ({
                      lat: location.position?.lat ?? 0,
                      lng: location.position?.lng ?? 0,
                    })),
                    { maxZoom: 18 },
                  );
              }}
            />
            {isOpen &&
              members.map((location, index) =>
                marker(
                  {
                    ...location,
                    position: { ...location.position, lat, lng },
                  } as geo.FriendLocation,
                  fanOffset(index, members.length),
                ),
              )}
          </ClusterGroup>
        );
      })}
      {(gatherings ?? []).map((gathering) =>
        gathering.center ? (
          <GatheringMarker
            key={gathering.id}
            lat={gathering.center.lat}
            lng={gathering.center.lng}
            members={gathering.members
              .map((member) => member.user)
              .filter((user): user is common.UserSummary => Boolean(user))}
            party={!gathering.atPrivatePlace}
            onSelect={() => {
              open({ kind: "gathering", id: gathering.id });
              if (gathering.center) flyTo(gathering.center.lng, gathering.center.lat, 16);
            }}
          />
        ) : null,
      )}
    </>
  );
}

function ClusterGroup({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

/** My own pin. Tapping it opens my sheet (sharing, ghost mode, who looked). */
function OwnMarker() {
  const { t } = useTranslation("map");
  const me = useMe();
  const own = useLocations((state) => state.own);
  const { data: ghost } = useGhostSettings();
  const { data: sos } = useActiveSos();
  const now = useNow(60_000);
  const open = useMapUi((state) => state.open);
  if (!own || !me?.user) return null;
  const hidden = ghostSummary(ghost, now) !== null;
  return (
    <MapPin lng={own.lng} lat={own.lat} zIndex={8}>
      <button
        type="button"
        onClick={() => open({ kind: "me" })}
        data-testid="own-pin"
        aria-label={t("me.open")}
        className="group flex flex-col items-center outline-none"
      >
        <span className="relative block transition-transform duration-200 ease-bounce group-hover:scale-105">
          <span
            className={cn(
              "absolute inset-0 animate-pulse-ring rounded-[1.25rem]",
              sos?.mine ? "bg-danger" : "bg-brand",
            )}
            aria-hidden
          />
          <span
            className={cn(
              "sticker relative block rounded-[1.25rem] p-[3px] group-focus-visible:ring-4 group-focus-visible:ring-brand/50",
              sos?.mine ? "bg-danger" : "bg-brand",
            )}
          >
            <Avatar user={me.user} size="lg" className="block [&>span]:rounded-[1rem]" />
          </span>
          {hidden && (
            <span className="sticker-sm absolute -top-3 -left-3 grid size-7 place-items-center rounded-full bg-ink text-surface">
              <Ghost className="size-3.5" />
            </span>
          )}
        </span>
        <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
        <span className="rounded-full bg-brand px-2.5 py-0.5 text-[11px] font-black text-on-brand">
          {sos?.mine ? "SOS" : t("you")}
        </span>
      </button>
    </MapPin>
  );
}

/** Everything along the top: where the map is looking, the story tray, and what is going on right now. */
function TopChrome() {
  const ref = useCoveredInset<HTMLDivElement>("top");
  return (
    <div className="pointer-events-none absolute inset-x-0 top-0 z-20 md:left-[25.5rem]">
      <div ref={ref} className="flex flex-col gap-2 p-4 pt-safe pr-20 md:p-6 md:pr-24">
        <MapTitle />
        <StoryTray />
        <div className="flex flex-wrap items-start gap-2">
          <GhostIndicator />
          <TripChips />
        </div>
        <SharingNotice />
        <SosBanners />
      </div>
    </div>
  );
}

/** Big place name of whatever the map is centred on; "World" when zoomed out. */
function MapTitle() {
  const { t, i18n } = useTranslation("map");
  const map = useMap();
  const [view, setView] = useState(() => ({
    lat: map.getCenter().lat,
    lng: map.getCenter().lng,
    zoom: map.getZoom(),
  }));
  useEffect(() => {
    const update = () =>
      setView({ lat: map.getCenter().lat, lng: map.getCenter().lng, zoom: map.getZoom() });
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
    queryFn: () =>
      placeClient().reverseGeocode({ position: { lat, lng }, language: i18n.language }),
    enabled: !zoomedOut,
    staleTime: 10 * 60_000,
    retry: false,
    placeholderData: (previous) => previous,
  });
  const name = zoomedOut ? t("world") : (data?.result?.name ?? "");
  if (!name) return <div className="h-10 md:h-12" />;
  return (
    <div>
      <h1
        key={name}
        className="animate-slide-up truncate text-4xl leading-tight font-black tracking-tight text-ink drop-shadow-[0_2px_0_var(--surface)] md:text-5xl"
      >
        {name}
      </h1>
      {!zoomedOut && data?.result?.region && data.result.region !== name && (
        <p className="text-sm font-bold text-ink-soft drop-shadow-[0_1px_0_var(--surface)]">
          {data.result.region}
        </p>
      )}
    </div>
  );
}

/** Says why my own pin is missing, at the moment it matters, with the one thing to do about it. */
function SharingNotice() {
  const { t } = useTranslation("map");
  const state = useSharingState();
  const { start, confirmStop, stopFailed } = useSharing();
  if (state === "sharing" || state === "starting") return null;
  // This device stopped sending (a stop, a revoked permission) but the server never heard:
  // that outranks everything else, and stays until the server has been told.
  const failed = stopFailed && state !== "sessionEnded";
  const copy = failed ? "stopFailed" : state;
  const action = failed
    ? t("sharing.tellAgain")
    : state === "needsConsent" || state === "needsPermission"
      ? t("sharing.turnOn")
      : state === "stopped"
        ? t("sharing.resume")
        : state === "unavailable"
          ? t("sharing.retry")
          : state === "sessionEnded"
            ? t("sharing.reload")
            : null;
  const act = failed
    ? () => void confirmStop()
    : state === "sessionEnded"
      ? () => window.location.reload()
      : start;
  return (
    <div
      className={cn(
        "sticker-sm pointer-events-auto flex w-full max-w-md flex-wrap items-center gap-x-3 gap-y-2 rounded-2xl px-4 py-2.5",
        failed ? "bg-danger text-white" : "glass",
      )}
      role={failed ? "alert" : undefined}
      data-testid="sharing-notice"
      data-state={copy}
    >
      <span className="text-2xl" aria-hidden>
        {failed ? "⚠️" : state === "stopped" ? "⏸️" : state === "blocked" ? "🚫" : "📍"}
      </span>
      <p
        className={cn(
          "min-w-[11rem] flex-1 text-xs leading-relaxed font-bold",
          failed ? "text-white" : "text-ink-soft",
        )}
      >
        <span className={cn("block text-sm font-black", !failed && "text-ink")}>
          {t(`sharing.${copy}.title`)}
        </span>
        {t(`sharing.${copy}.body`)}
      </p>
      {action && (
        <button
          type="button"
          onClick={act}
          className="pressable sticker-sm ml-auto h-9 shrink-0 rounded-full bg-lime px-3.5 text-xs font-black text-[#17122b]"
        >
          {action}
        </button>
      )}
    </div>
  );
}

function RoundButton({
  label,
  active,
  className,
  children,
  ...props
}: {
  label: string;
  active?: boolean;
  className?: string;
  children: ReactNode;
  disabled?: boolean;
  onClick?(): void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active}
      className={cn(
        "sticker-sm pressable grid size-12 place-items-center rounded-full disabled:opacity-50",
        active ? "bg-ink text-surface" : "glass",
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

/** The column of round buttons: friends, locate me, everyone, ghost mode, more; compass and zoom where they help. */
function MapButtons() {
  const { t } = useTranslation("map");
  const map = useMap();
  const own = useLocations((state) => state.own);
  const flyTo = useFlyTo();
  const { open } = useMapUi();
  const { data: ghost } = useGhostSettings();
  const now = useNow(60_000);
  const rotated = useMapValue(
    (instance) => Math.abs(instance.getBearing()) > 1 || instance.getPitch() > 1,
    "rotate pitch",
  );
  const bearing = useMapValue((instance) => Math.round(instance.getBearing()), "rotate");

  const showEveryone = () => {
    const points = Object.values(useLocations.getState().friends)
      .filter((location) => location.position)
      .map((location) => ({ lat: location.position?.lat ?? 0, lng: location.position?.lng ?? 0 }));
    if (own) points.push({ lat: own.lat, lng: own.lng });
    fitMapToPoints(map, points, { maxZoom: 15 });
  };

  return (
    <div
      className="absolute top-0 right-0 z-20 flex flex-col gap-2 p-4 pt-safe md:p-6"
      role="toolbar"
      aria-orientation="vertical"
      aria-label={t("controls.label")}
    >
      <RoundButton label={t("list.title")} onClick={() => open({ kind: "friends" })}>
        <Users className="size-5" />
      </RoundButton>
      <RoundButton
        label={t("recenter")}
        disabled={!own}
        onClick={() => own && flyTo(own.lng, own.lat, 16)}
      >
        <LocateFixed className="size-5" />
      </RoundButton>
      <RoundButton label={t("showEveryone")} onClick={showEveryone}>
        <span className="text-lg leading-none" aria-hidden>
          🌏
        </span>
      </RoundButton>
      <RoundButton
        label={t("ghost.title")}
        active={ghostSummary(ghost, now) !== null}
        onClick={() => open({ kind: "ghost" })}
      >
        <Ghost className="size-5" />
      </RoundButton>
      <Menu
        trigger={
          <button
            type="button"
            aria-label={t("controls.more")}
            title={t("controls.more")}
            className="glass sticker-sm pressable grid size-12 place-items-center rounded-full"
          >
            <MoreHorizontal className="size-5" />
          </button>
        }
        actions={[
          { label: t("me.menu"), icon: <UserRound />, onSelect: () => open({ kind: "me" }) },
          {
            label: t("trips.title"),
            icon: <Navigation />,
            onSelect: () => open({ kind: "trips" }),
          },
          {
            label: t("safety.title"),
            icon: <BellRing />,
            onSelect: () => open({ kind: "safety" }),
          },
          { label: t("shares.title"), icon: <Link2 />, onSelect: () => open({ kind: "shares" }) },
        ]}
      />
      {rotated && (
        <RoundButton
          label={t("controls.north")}
          onClick={() => map.easeTo({ bearing: 0, pitch: 0, duration: 400 })}
        >
          <Compass
            className="size-5 transition-transform"
            style={{ transform: `rotate(${-bearing - 45}deg)` }}
          />
        </RoundButton>
      )}
      <div className="hidden flex-col gap-2 md:flex">
        <RoundButton label={t("controls.zoomIn")} onClick={() => map.zoomIn()}>
          <Plus className="size-5" />
        </RoundButton>
        <RoundButton label={t("controls.zoomOut")} onClick={() => map.zoomOut()}>
          <Minus className="size-5" />
        </RoundButton>
      </div>
    </div>
  );
}

/** On phones, with no sheet open: the row of friends to fly to — or the invitation to add some. */
function BottomChrome() {
  const { t } = useTranslation("map");
  const wide = useIsWide();
  const sheetOpen = useMapUi((state) => state.stack.length > 0);
  const friends = useFriendsById();
  const locations = useLocations((state) => state.friends);
  const online = useLocations((state) => state.online);
  const own = useLocations((state) => state.own);
  const { data: ghost } = useGhostSettings();
  const flyToUser = useFlyToUser();
  const { open } = useMapUi();
  const now = useNow(30_000);
  const shown = !wide && !sheetOpen;
  const ref = useCoveredInset<HTMLDivElement>("bottom", shown);
  if (!shown) return null;

  const entries = Object.values(locations)
    .filter((location) => location.position && friends[location.userId]?.user)
    .map((location) => {
      const friend = friends[location.userId];
      const mutual = seeEachOther(location, ghostRuleFor(ghost, location.userId, now), now);
      return {
        location,
        friend,
        user: friend?.user as common.UserSummary,
        distance: mutual ? distanceTo(own, location) : null,
      };
    })
    .sort(
      (a, b) => (a.distance ?? Number.POSITIVE_INFINITY) - (b.distance ?? Number.POSITIVE_INFINITY),
    );

  if (Object.keys(friends).length === 0) {
    return (
      <div className="absolute inset-x-0 bottom-[5.5rem] z-20 flex justify-center px-4">
        <div
          ref={ref}
          className="glass sticker flex w-full max-w-sm items-center gap-3 rounded-3xl p-3"
          data-testid="map-empty"
        >
          <span className="animate-float text-4xl" aria-hidden>
            🫡
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-sm leading-snug font-black [word-break:keep-all]">
              {t("noFriendsTitle")}
            </p>
            <p className="text-xs leading-snug text-ink-soft [word-break:keep-all]">
              {t("noFriendsBody")}
            </p>
          </div>
          <Link
            to="/friends"
            className="pressable sticker-sm flex h-10 shrink-0 items-center justify-center rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
          >
            {t("addFriends")}
          </Link>
        </div>
      </div>
    );
  }
  if (entries.length === 0) return null;
  return (
    <div className="absolute inset-x-0 bottom-[5.5rem] z-20 flex justify-center">
      <div
        ref={ref}
        className="scrollbar-none flex max-w-full gap-3 overflow-x-auto px-4 py-2"
        data-testid="friend-carousel"
      >
        {entries.map(({ location, friend, user, distance }) => (
          <button
            key={user.id}
            type="button"
            aria-label={friendName(friend, user)}
            onClick={() => {
              open({ kind: "friend", userId: user.id });
              flyToUser(user.id);
            }}
            className={cn(
              "pressable flex w-16 shrink-0 flex-col items-center gap-1",
              pinState(location, now) !== "live" && "opacity-70",
            )}
          >
            <Avatar user={user} size="lg" outlined online={online[user.id]} />
            <span className="glass w-full truncate rounded-full px-1.5 text-center text-[10px] font-black">
              {distance !== null ? formatDistance(distance) : friendName(friend, user)}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** Shows the open panel: a sheet on phones, the side panel on wide screens (where the friend list is always there). */
function PanelHost() {
  const wide = useIsWide();
  const top = useMapUi((state) => state.stack.at(-1));
  const panel = top ?? (wide ? ({ kind: "friends" } as const) : null);
  if (!panel) return null;
  switch (panel.kind) {
    case "friends":
      return <FriendListPanel />;
    case "friend":
      return <FriendSheet key={panel.userId} userId={panel.userId} />;
    case "gathering":
      return <GatheringSheet key={panel.id} id={panel.id} />;
    case "me":
      return <MeSheet />;
    case "ghost":
      return <GhostPanel />;
    case "trips":
      return <TripsPanel />;
    case "safety":
      return <SafetyPanel />;
    case "shares":
      return <SharesPanel />;
    case "story":
      return <StoryPanel key={panel.userId} userId={panel.userId} />;
  }
}
