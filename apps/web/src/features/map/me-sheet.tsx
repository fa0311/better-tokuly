import { useQuery } from "@tanstack/react-query";
import { geo } from "@tokuly/api";
import { BellRing, ChevronRight, Ghost, Link2, MapPinOff, Navigation, Play } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { useMe } from "#/stores/session";
import {
  mapKeys,
  mapStoryClient,
  useActiveTrips,
  useGhostSettings,
  useLiveShares,
  useMapViews,
  useNow,
  usePlaceAlerts,
} from "./data";
import { ghostSummary } from "./model";
import { useSharing, useSharingState } from "./own-location";
import { MapSheet, SheetSection } from "./sheet";
import { type Panel, useMapUi } from "./ui-store";

/** What each sharing state reads as, and what the button offers. */
export function SharingStatus() {
  const { t } = useTranslation("map");
  const state = useSharingState();
  const { start, stop, confirmStop, stopFailed } = useSharing();
  return (
    <div className="rounded-2xl bg-surface-2 p-3" data-testid="sharing-status" data-state={state}>
      <p className="flex items-center gap-2 text-sm font-black">
        <span
          className={
            state === "sharing"
              ? "size-2.5 rounded-full bg-success"
              : "size-2.5 rounded-full bg-ink-faint"
          }
          aria-hidden
        />
        {t(`sharing.${state}.title`)}
      </p>
      <p className="mt-1 text-xs leading-relaxed font-bold text-ink-soft">
        {t(`sharing.${state}.body`)}
      </p>
      {(state === "sharing" || state === "starting") && (
        <Button
          className="mt-2"
          size="sm"
          variant="secondary"
          icon={<MapPinOff className="size-4" />}
          onClick={() => void stop()}
        >
          {t("sharing.stop")}
        </Button>
      )}
      {state === "stopped" && stopFailed && (
        <div className="mt-2 rounded-xl bg-danger/15 p-2.5" role="alert" data-testid="stop-failed">
          <p className="text-xs leading-relaxed font-bold text-danger">
            {t("sharing.stopFailed.title")} {t("sharing.stopFailed.body")}
          </p>
          <Button className="mt-2" size="sm" variant="danger" onClick={() => void confirmStop()}>
            {t("sharing.tellAgain")}
          </Button>
        </div>
      )}
      {(state === "stopped" ||
        state === "needsConsent" ||
        state === "needsPermission" ||
        state === "unavailable") && (
        <Button className="mt-2" size="sm" variant="lime" onClick={start}>
          {state === "stopped"
            ? t("sharing.resume")
            : state === "unavailable"
              ? t("sharing.retry")
              : t("sharing.turnOn")}
        </Button>
      )}
    </div>
  );
}

function HubRow({
  icon,
  label,
  value,
  panel,
}: {
  icon: ReactNode;
  label: string;
  value?: ReactNode;
  panel: Panel;
}) {
  const push = useMapUi((state) => state.push);
  return (
    <li>
      <button
        type="button"
        onClick={() => push(panel)}
        className="flex w-full items-center gap-3 rounded-2xl px-2 py-2.5 text-left hover:bg-surface-2"
      >
        <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-surface-2">
          {icon}
        </span>
        <span className="min-w-0 flex-1 truncate font-bold">{label}</span>
        {value && <span className="shrink-0 text-xs font-bold text-ink-soft">{value}</span>}
        <ChevronRight className="size-4 shrink-0 text-ink-faint" />
      </button>
    </li>
  );
}

/** My own sheet: whether I am sharing, who looked, and the way to everything about my own position. */
export function MeSheet() {
  const { t } = useTranslation("map");
  const me = useMe();
  const { stack, back, close, push } = useMapUi();
  const { data: views } = useMapViews();
  const { data: ghost } = useGhostSettings();
  const { data: trips } = useActiveTrips();
  const { data: shares } = useLiveShares();
  const { data: alerts } = usePlaceAlerts();
  const now = useNow(60_000);
  const myId = me?.user?.id ?? "";
  const { data: reel } = useQuery({
    queryKey: mapKeys.storyReel(myId),
    queryFn: async () => (await mapStoryClient().listMapStories({ userId: myId })).myReel ?? null,
    enabled: Boolean(myId),
    staleTime: 60_000,
  });
  const summary = ghostSummary(ghost, now);
  const pendingAlerts =
    alerts?.aboutMe.filter((alert) => alert.status === geo.AlertStatus.PENDING_APPROVAL).length ??
    0;

  return (
    <MapSheet
      testId="me-sheet"
      title={me?.user?.displayName ?? t("you")}
      subtitle={t("me.subtitle")}
      leading={<Avatar user={me?.user} size="md" outlined />}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      <SharingStatus />

      <SheetSection title={t("me.views")}>
        <div
          className="flex items-center gap-3 rounded-2xl bg-surface-2 p-3"
          data-testid="map-views"
        >
          <span className="text-2xl" aria-hidden>
            👀
          </span>
          <p className="min-w-0 flex-1 text-sm font-bold">
            {views?.viewerCount ? t("me.viewers", { count: views.viewerCount }) : t("me.noViewers")}
          </p>
          <span className="flex -space-x-2">
            {views?.viewers.slice(0, 5).map((viewer) => (
              <Avatar key={viewer.id} user={viewer} size="xs" outlined />
            ))}
          </span>
        </div>
      </SheetSection>

      <SheetSection>
        <ul>
          <HubRow
            icon={<Ghost className="size-5 text-brand" />}
            label={t("ghost.title")}
            value={summary ? t("ghost.onShort") : t("ghost.offShort")}
            panel={{ kind: "ghost" }}
          />
          <HubRow
            icon={<Navigation className="size-5 text-sky" />}
            label={t("trips.title")}
            value={trips?.length ? t("me.count", { count: trips.length }) : undefined}
            panel={{ kind: "trips" }}
          />
          <HubRow
            icon={<BellRing className="size-5 text-coral" />}
            label={t("safety.title")}
            value={pendingAlerts ? t("me.waiting", { count: pendingAlerts }) : undefined}
            panel={{ kind: "safety" }}
          />
          <HubRow
            icon={<Link2 className="size-5 text-success" />}
            label={t("shares.title")}
            value={shares?.length ? t("me.count", { count: shares.length }) : undefined}
            panel={{ kind: "shares" }}
          />
        </ul>
      </SheetSection>

      {reel && reel.items.length > 0 && (
        <SheetSection title={t("stories.mine")}>
          <Button
            block
            variant="secondary"
            icon={<Play className="size-4" />}
            onClick={() => push({ kind: "story", userId: myId })}
          >
            {t("stories.playMine", { count: reel.items.length })}
          </Button>
        </SheetSection>
      )}
    </MapSheet>
  );
}
