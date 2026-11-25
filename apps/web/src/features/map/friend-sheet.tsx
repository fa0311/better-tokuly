import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common, geo, toMillis } from "@tokuly/api";
import {
  BUMP_RADIUS_M,
  BUZZ_RADIUS_M,
  distanceTo,
  ghostRuleFor,
  isBlurred,
  isMoving,
  pinState,
  seeEachOther,
  stayMillis,
} from "@tokuly/core/map";
import {
  BellRing,
  Flag,
  Hand,
  MessageCircle,
  Navigation,
  UserRound,
  Vibrate,
  Zap,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Skeleton } from "#/components/ui/page";
import { Spinner } from "#/components/ui/spinner";
import { openDirectChat } from "#/features/chat";
import { ReportDialog } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge, compactDuration, formatDistance, kmh } from "#/lib/format";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { tryBump } from "./bump";
import {
  friendName,
  interactionClient,
  locationClient,
  mapKeys,
  PLACE_LABEL_KEYS,
  tripClient,
  useFriendsById,
  useGhostSettings,
  useNow,
} from "./data";
import { ACTIVITY_EMOJI, ACTIVITY_KEYS, BatteryBadge } from "./pins";
import { ReactionBar } from "./reaction-bar";
import { CreateAlertDialog } from "./safety";
import { Fact, MapSheet, SheetSection } from "./sheet";
import { StartTripDialog, TRAVEL_MODES } from "./trips";
import { useMapUi } from "./ui-store";

/** "8分" / "1時間20分" for a travel time. */
export function formatTravelTime(seconds: number, language: string): string {
  const minutes = Math.max(1, Math.round(seconds / 60));
  const ja = language.startsWith("ja");
  if (minutes < 60) return ja ? `${minutes}分` : `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (ja) return rest ? `${hours}時間${rest}分` : `${hours}時間`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/** Where someone is, in words: the badge for home/work/school, a shared place, or the area. */
export function usePlaceText(): (location: geo.FriendLocation | undefined) => string {
  const { t } = useTranslation("map");
  return (location) => {
    if (!location) return "";
    const label =
      location.precision === geo.GhostMode.BLURRED
        ? undefined
        : PLACE_LABEL_KEYS[location.placeLabel];
    if (label) return t(`place.${label}`);
    if (location.placeName) return `${location.placeEmoji} ${location.placeName}`.trim();
    return location.locality;
  };
}

/** The panel for a friend: who, where, how they are doing, and everything you can do with them. */
export function FriendSheet({ userId }: { userId: string }) {
  const { t, i18n } = useTranslation("map");
  const friend = useFriendsById()[userId];
  const location = useLocations((state) => state.friends[userId]);
  const online = useLocations((state) => state.online[userId]);
  const own = useLocations((state) => state.own);
  const { data: ghost } = useGhostSettings();
  const now = useNow(10_000);
  const placeText = usePlaceText();
  const { stack, back, close, push } = useMapUi();
  const [mode, setMode] = useState(geo.TravelMode.WALK);
  const [tripOpen, setTripOpen] = useState(false);
  const [alertOpen, setAlertOpen] = useState(false);
  const [reportOpen, setReportOpen] = useState(false);
  const user = friend?.user;

  // Opening someone's pin is a "map view": they see that somebody looked.
  useEffect(() => {
    void interactionClient()
      .recordMapView({ userId })
      .catch(() => undefined);
  }, [userId]);

  const myRule = ghostRuleFor(ghost, userId, now);
  const mutual = seeEachOther(location, myRule, now);
  const distance = mutual ? distanceTo(own, location) : null;
  const directions = useQuery({
    queryKey: mapKeys.directions(userId, mode),
    queryFn: () =>
      tripClient().getDirections({
        destination: { target: { case: "userId", value: userId } },
        mode,
      }),
    enabled: mutual && Boolean(own),
    staleTime: 60_000,
    retry: false,
  });

  if (!user) {
    return (
      <MapSheet title={t("friend.gone")} onClose={close}>
        <p className="py-6 text-center text-sm text-ink-soft">{t("friend.goneBody")}</p>
      </MapSheet>
    );
  }

  const name = friendName(friend, user);
  const state = location ? pinState(location, now) : null;
  const blurred = Boolean(location && isBlurred(location));
  const place = placeText(location);
  const stay = location ? stayMillis(location, now) : null;
  const status = location?.status ?? friend.status;

  return (
    <MapSheet
      testId="friend-sheet"
      title={name}
      subtitle={`@${user.handle}`}
      leading={<Avatar user={user} size="md" outlined online={online} />}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      {(status?.emoji || status?.text) && (
        <p className="truncate text-sm">
          {status.emoji} {status.text}
        </p>
      )}

      {/* Where they are. */}
      <div className="mt-2 rounded-2xl bg-surface-2 px-4 py-3" data-testid="friend-where">
        {!location?.position ? (
          <p className="text-sm font-bold text-ink-soft">{t("friend.noLocation")}</p>
        ) : (
          <>
            <p className="flex items-center gap-2 text-sm font-black">
              <span className="min-w-0 flex-1 truncate">📍 {place || t("friend.somewhere")}</span>
              {distance !== null && (
                <span className="shrink-0 text-ink-soft">
                  {t("distanceAway", { distance: formatDistance(distance) })}
                </span>
              )}
            </p>
            {blurred && (
              <p className="mt-1 text-xs leading-relaxed font-bold text-ink-soft">
                {t("friend.blurred")}
              </p>
            )}
            {state === "stale" && (
              <p className="mt-1 text-xs leading-relaxed font-bold text-ink-soft">
                {t("friend.stale", { age: compactAge(toMillis(location.recordedAt), now) })}
              </p>
            )}
            {state === "paused" && (
              <p className="mt-1 text-xs leading-relaxed font-bold text-ink-soft">
                {t("friend.paused")}
              </p>
            )}
            {mutual && own && (
              <div className="mt-2 flex items-center gap-2">
                <div className="flex gap-1" role="radiogroup" aria-label={t("travel.mode")}>
                  {TRAVEL_MODES.map(({ mode: option, key, icon: Icon }) => (
                    // biome-ignore lint/a11y/useSemanticElements: a row of icon toggles reads better than radio inputs here.
                    <button
                      key={key}
                      type="button"
                      role="radio"
                      aria-checked={mode === option}
                      aria-label={t(`travel.${key}`)}
                      title={t(`travel.${key}`)}
                      onClick={() => setMode(option)}
                      className={cn(
                        "grid size-8 place-items-center rounded-full",
                        mode === option
                          ? "sticker-sm bg-brand text-on-brand"
                          : "text-ink-soft hover:bg-surface-3",
                      )}
                    >
                      <Icon className="size-4" />
                    </button>
                  ))}
                </div>
                <div
                  className="min-w-0 flex-1 truncate text-right text-sm font-black"
                  data-testid="travel-time"
                >
                  {directions.isPending ? (
                    <Skeleton className="ml-auto h-5 w-24" />
                  ) : directions.data ? (
                    <>
                      {directions.data.provider === "estimate"
                        ? t("travel.about", {
                            time: formatTravelTime(directions.data.durationSeconds, i18n.language),
                          })
                        : formatTravelTime(directions.data.durationSeconds, i18n.language)}
                      <span className="ml-1.5 font-bold text-ink-soft">
                        {formatDistance(directions.data.distanceMeters)}
                      </span>
                    </>
                  ) : (
                    <span className="text-ink-soft">{t("travel.unknown")}</span>
                  )}
                </div>
              </div>
            )}
          </>
        )}
      </div>

      {location?.position && state !== "paused" && (
        <ul className="mt-2 flex flex-wrap gap-1.5" data-testid="friend-facts">
          {location.sleeping && <Fact>😴 {t("sleeping")}</Fact>}
          {state === "live" && isMoving(location, now) && (
            <Fact>
              {ACTIVITY_EMOJI[location.activity] ?? "💨"}{" "}
              {ACTIVITY_KEYS[location.activity]
                ? t(`activity.${ACTIVITY_KEYS[location.activity]}`)
                : t("activity.moving")}{" "}
              · {kmh(location.speedMps)} km/h
            </Fact>
          )}
          {state === "live" && stay !== null && (
            <Fact>⏱ {t("stationaryFor", { duration: compactDuration(stay) })}</Fact>
          )}
          {location.batteryPercent !== undefined && (
            <Fact
              tone={
                location.batteryPercent <= 20 && !location.charging
                  ? "warn"
                  : location.charging
                    ? "good"
                    : "plain"
              }
            >
              <BatteryBadge
                percent={location.batteryPercent}
                charging={location.charging}
                className="h-5 !shadow-none"
              />
              {location.charging
                ? t("charging")
                : location.batteryPercent <= 20
                  ? t("batteryLow")
                  : t("batteryOk")}
            </Fact>
          )}
          {location.temperatureCelsius !== undefined && (
            <Fact>🌡 {Math.round(location.temperatureCelsius)}°C</Fact>
          )}
          {location.timeZone &&
            location.timeZone !== Intl.DateTimeFormat().resolvedOptions().timeZone && (
              <Fact>
                🕐{" "}
                {t("localTime", {
                  time: new Intl.DateTimeFormat(i18n.language, {
                    hour: "2-digit",
                    minute: "2-digit",
                    timeZone: location.timeZone,
                  }).format(now),
                })}
              </Fact>
            )}
          <Fact>
            {blurred
              ? t("lastSeenAbout", { age: compactAge(toMillis(location.recordedAt), now) })
              : t("lastSeen", { age: compactAge(toMillis(location.recordedAt), now) })}
          </Fact>
        </ul>
      )}

      <SheetSection title={t("reactions.title")}>
        <ReactionBar userIds={[userId]} />
      </SheetSection>

      <SheetSection>
        <div className="grid grid-cols-4 gap-2">
          <ActionTile
            icon={<MessageCircle className="size-5" />}
            label={t("actions.chat")}
            tone="brand"
            onClick={() => void openDirectChat(user.id)}
          />
          <ActionTile
            icon={<Navigation className="size-5" />}
            label={t("actions.trip")}
            unavailable={!mutual}
            onClick={() => (mutual ? setTripOpen(true) : toast(t("rules.trip"), { icon: "🧭" }))}
          />
          <ActionTile
            icon={<BellRing className="size-5" />}
            label={t("actions.alert")}
            onClick={() => setAlertOpen(true)}
          />
          <Link
            to="/u/$handle"
            params={{ handle: user.handle }}
            className="pressable sticker-sm flex h-16 flex-col items-center justify-center gap-1 rounded-2xl bg-surface text-[11px] font-black"
          >
            <UserRound className="size-5" />
            {t("actions.profile")}
          </Link>
        </div>
        {!mutual && location?.position && (
          <p className="mt-1.5 text-[11px] leading-relaxed font-bold text-ink-faint">
            {myRule !== geo.GhostMode.PRECISE ? t("rules.hiddenFromThem") : t("rules.trip")}
          </p>
        )}
      </SheetSection>

      <SheetSection title={t("nudge.title")}>
        <NudgeRow userId={userId} name={name} mutual={mutual} distance={distance} />
      </SheetSection>

      <BumpHistory userId={userId} />

      <SheetSection
        title={t("ghost.forThisFriend", { name })}
        action={
          <TextButton onClick={() => push({ kind: "ghost" })}>{t("ghost.openSettings")}</TextButton>
        }
      >
        <GhostQuickPick userIds={[userId]} current={myRule} />
      </SheetSection>

      <div className="mt-4 flex justify-center">
        <TextButton onClick={() => setReportOpen(true)}>
          <Flag className="size-3.5" /> {t("actions.report")}
        </TextButton>
      </div>

      <StartTripDialog
        open={tripOpen}
        onOpenChange={setTripOpen}
        destination={{ kind: "friend", userId, name }}
        initialMode={mode}
      />
      <CreateAlertDialog open={alertOpen} onOpenChange={setAlertOpen} subjectId={userId} />
      <ReportDialog
        subject={{ type: common.SubjectType.USER, id: user.id }}
        open={reportOpen}
        onOpenChange={setReportOpen}
        label={`@${user.handle}`}
      />
    </MapSheet>
  );
}

export function TextButton({ children, onClick }: { children: ReactNode; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex shrink-0 items-center gap-1 rounded-full px-2 py-1 text-xs font-bold text-ink-soft hover:bg-surface-2 hover:text-ink"
    >
      {children}
    </button>
  );
}

function ActionTile({
  icon,
  label,
  tone,
  unavailable,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  tone?: "brand";
  unavailable?: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-disabled={unavailable || undefined}
      className={cn(
        "pressable sticker-sm flex h-16 flex-col items-center justify-center gap-1 rounded-2xl text-[11px] font-black",
        tone === "brand" ? "bg-brand text-on-brand" : "bg-surface",
        unavailable && "opacity-55",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

/**
 * Knock taps stack until the finger pauses this long. People knock three to
 * five times a second, so a burst holds together on any phone, and the knock
 * still goes out soon after the last tap.
 */
const KNOCK_BURST_PAUSE_MS = 700;

/** Bump, buzz and knock, each saying when it works. */
function NudgeRow({
  userId,
  name,
  mutual,
  distance,
}: {
  userId: string;
  name: string;
  mutual: boolean;
  distance: number | null;
}) {
  const { t } = useTranslation("map");
  const myId = useMe()?.user?.id;
  const [knocks, setKnocks] = useState(0);
  const knockTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pendingKnocks = useRef(0);
  useEffect(() => () => clearTimeout(knockTimer.current), []);

  const near = (radius: number) => mutual && distance !== null && distance <= radius;
  const farText = () =>
    !mutual
      ? t("rules.needPrecise")
      : distance === null
        ? t("rules.needLocation")
        : t("rules.tooFar", { distance: formatDistance(distance) });

  const buzz = useMutation({
    mutationFn: () => interactionClient().buzz({ userId }),
    onSuccess: () => toast.success(t("nudge.buzzSent", { name })),
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  // Quick taps stack into one knock burst, like knocking on a door.
  const knock = () => {
    pendingKnocks.current = Math.min(20, pendingKnocks.current + 1);
    setKnocks(pendingKnocks.current);
    clearTimeout(knockTimer.current);
    knockTimer.current = setTimeout(async () => {
      const count = pendingKnocks.current;
      pendingKnocks.current = 0;
      setKnocks(0);
      try {
        await interactionClient().knock({ userId, count });
        toast.success(t("nudge.knockSent", { name, count }));
      } catch (error) {
        toast.error(errorMessage(error, { namespace: "map" }));
      }
    }, KNOCK_BURST_PAUSE_MS);
  };

  const bumpOk = near(BUMP_RADIUS_M);
  const buzzOk = near(BUZZ_RADIUS_M);
  return (
    <div className="grid grid-cols-3 gap-2">
      <NudgeTile
        testId="nudge-bump"
        icon={<Zap className="size-5 text-sun" />}
        label={t("nudge.bump")}
        hint={bumpOk ? t("nudge.bumpHint") : `${t("nudge.bumpRule")} ${farText()}`}
        unavailable={!bumpOk}
        onClick={() =>
          bumpOk ? void tryBump(myId) : toast(`${t("nudge.bumpRule")} ${farText()}`, { icon: "⚡" })
        }
      />
      <NudgeTile
        testId="nudge-buzz"
        icon={<Vibrate className="size-5 text-brand" />}
        label={t("nudge.buzz")}
        hint={buzzOk ? t("nudge.buzzHint") : `${t("nudge.buzzRule")} ${farText()}`}
        unavailable={!buzzOk}
        busy={buzz.isPending}
        onClick={() =>
          buzzOk ? buzz.mutate() : toast(`${t("nudge.buzzRule")} ${farText()}`, { icon: "📳" })
        }
      />
      <NudgeTile
        testId="nudge-knock"
        icon={<Hand className="size-5 text-coral" />}
        label={knocks > 0 ? `${t("nudge.knock")} ×${knocks}` : t("nudge.knock")}
        hint={t("nudge.knockHint")}
        onClick={knock}
      />
    </div>
  );
}

function NudgeTile({
  icon,
  label,
  hint,
  unavailable,
  busy,
  onClick,
  testId,
}: {
  icon: ReactNode;
  label: string;
  hint: string;
  unavailable?: boolean;
  busy?: boolean;
  onClick(): void;
  testId: string;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      aria-disabled={unavailable || undefined}
      aria-busy={busy || undefined}
      className={cn(
        "pressable flex flex-col items-center gap-1 rounded-2xl bg-surface-2 px-1.5 py-2.5 text-center",
        unavailable && "bg-transparent outline-2 -outline-offset-2 outline-line outline-dashed",
      )}
    >
      <span className={cn(unavailable && "opacity-50")}>{icon}</span>
      <span className="text-xs font-black">{label}</span>
      <span className="text-[10px] leading-snug font-bold text-ink-soft">{hint}</span>
    </button>
  );
}

function BumpHistory({ userId }: { userId: string }) {
  const { t } = useTranslation("map");
  const { data } = useQuery({
    queryKey: mapKeys.bumps(userId),
    queryFn: async () =>
      (await interactionClient().listBumps({ withUserId: userId, page: { limit: 3 } })).bumps,
    staleTime: 60_000,
  });
  if (!data?.length) return null;
  return (
    <SheetSection title={t("bump.history", { count: data[0]?.pairCount ?? data.length })}>
      <ul className="space-y-1">
        {data.map((bump) => (
          <li
            key={bump.id}
            className="flex items-center gap-2 rounded-xl bg-surface-2 px-3 py-1.5 text-xs font-bold"
          >
            <span aria-hidden>⚡</span>
            <span className="min-w-0 flex-1 truncate">{bump.placeName || t("bump.somewhere")}</span>
            <span className="text-ink-soft">{compactAge(toMillis(bump.createdAt))}</span>
          </li>
        ))}
      </ul>
    </SheetSection>
  );
}

export const GHOST_MODES = [
  { mode: geo.GhostMode.PRECISE, key: "precise", emoji: "📍" },
  { mode: geo.GhostMode.BLURRED, key: "blurred", emoji: "🌫️" },
  { mode: geo.GhostMode.FROZEN, key: "frozen", emoji: "🧊" },
] as const;

/** One tap sets how precisely the given friends (or, with none, everyone) see me. */
export function GhostQuickPick({
  userIds,
  current,
  durationSeconds = 0,
}: {
  userIds: string[];
  current: geo.GhostMode;
  durationSeconds?: number;
}) {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const set = useMutation({
    mutationFn: (mode: geo.GhostMode) =>
      locationClient().setGhostMode({ userIds, mode, durationSeconds }),
    onSuccess: (response, mode) => {
      if (response.settings) queryClient.setQueryData(mapKeys.ghost, response.settings);
      toast.success(t(`ghost.set.${GHOST_MODES.find((entry) => entry.mode === mode)?.key}`), {
        id: "ghost-set",
      });
    },
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "map" }));
      // Whatever happened, show what the server has now.
      void queryClient.invalidateQueries({ queryKey: mapKeys.ghost });
    },
  });
  // The selected card is what the server has. A change in flight shows as "working" on its
  // card and only becomes the selection once the server agreed.
  const pending = set.isPending ? set.variables : undefined;
  return (
    <div className="grid grid-cols-3 gap-1.5" role="radiogroup" aria-label={t("ghost.title")}>
      {GHOST_MODES.map(({ mode, key, emoji }) => (
        // biome-ignore lint/a11y/useSemanticElements: big tappable cards, not radio inputs.
        <button
          key={key}
          type="button"
          role="radio"
          aria-checked={current === mode}
          aria-busy={pending === mode || undefined}
          data-testid={`ghost-${key}`}
          disabled={set.isPending}
          onClick={() => current !== mode && set.mutate(mode)}
          className={cn(
            "pressable relative flex flex-col items-center gap-0.5 rounded-2xl px-1 py-2 text-center disabled:opacity-100",
            current === mode ? "sticker-sm bg-brand text-on-brand" : "bg-surface-2",
            pending === mode && "outline-2 outline-brand outline-dashed",
          )}
        >
          {pending === mode && <Spinner className="absolute top-1.5 right-1.5 size-3.5" />}
          <span className="text-xl" aria-hidden>
            {emoji}
          </span>
          <span className="text-xs font-black">{t(`ghost.mode.${key}`)}</span>
        </button>
      ))}
    </div>
  );
}
