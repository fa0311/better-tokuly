import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Code, errorReason, geo, isApiError } from "@tokuly/api";
import { Gift } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Page, Skeleton } from "#/components/ui/page";
import {
  DEFAULT_CENTER,
  GazetteerCredit,
  type LatLng,
  locationClient,
  toLatLng,
  useMap,
} from "#/features/places";
import { profileClient } from "#/lib/api";
import { useMe } from "#/stores/session";
import { footprintClient, footprintKeys, regionName } from "./api";
import { celebrate } from "./celebrate";
import { FootprintMap } from "./footprint-map";
import { Leaderboard } from "./leaderboard";
import { explorerLevel, formatPercent, levelUps, regionLevel } from "./levels";
import { Meter } from "./meter";
import { RecapDialog, RecapTeaser, useRecap } from "./recap";
import { RegionList } from "./region-list";
import { readSeenLevels, regionMark, writeSeenLevels } from "./seen-levels";

/**
 * `/footprints`: the scratch map as a game. My own map with levels, the
 * recap and the ranking among friends; with `userId`, a friend's map and
 * numbers as far as they show them.
 */
export function FootprintsScreen({ userId }: { userId?: string }) {
  return userId ? <FriendFootprints key={userId} userId={userId} /> : <MyFootprints />;
}

function MyFootprints() {
  const { t } = useTranslation("footprints");
  const [recapOpen, setRecapOpen] = useState(false);
  const [boardRegion, setBoardRegion] = useState("world");
  const [boardScope, setBoardScope] = useState<"friends" | "global">("friends");
  const [levelUp, setLevelUp] = useState<string | null>(null);
  const userId = useMe()?.user?.id ?? "";

  const stats = useQuery({
    queryKey: footprintKeys.stats(""),
    queryFn: () => footprintClient().getStats({}),
  });
  const recap = useRecap();
  // Where the map opens: where the person last was, else the area they know best.
  const location = useQuery({
    queryKey: ["geo", "location", "mine"],
    queryFn: () => locationClient().getMyLocation({}),
    staleTime: 60_000,
    retry: false,
  });

  const total = Number(stats.data?.totalCellCount ?? 0);
  const regions = stats.data?.regions ?? [];
  const explorer = explorerLevel(total);

  // Announce levels that rose since the last look, once.
  const loaded = stats.isSuccess;
  const levelsKey = `${explorer.level}|${regions.map((stat) => `${stat.region?.id}:${regionLevel(stat.percent).level}`).join(",")}`;
  useEffect(() => {
    if (!loaded || !userId) return;
    // Regions are compared and remembered by their fingerprint for this account, never by ID.
    const marked = regions.flatMap((stat) =>
      stat.region
        ? [
            {
              id: regionMark(userId, stat.region.id),
              level: regionLevel(stat.percent).level,
              region: stat.region,
            },
          ]
        : [],
    );
    const ups = levelUps(readSeenLevels(userId), explorer.level, marked);
    writeSeenLevels(userId, ups.seen);
    const messages: string[] = [];
    if (ups.explorer) messages.push(t("levelUp.explorer", { level: ups.explorer }));
    for (const up of ups.regions) {
      const region = marked.find((entry) => entry.id === up.id)?.region;
      messages.push(t("levelUp.region", { region: regionName(region), level: up.level }));
    }
    if (messages.length === 0) return;
    celebrate(["🎉", "⭐", "🆙", "✨"]);
    setLevelUp(messages[0]!);
    for (const message of messages) toast.success(message);
  }, [loaded, levelsKey, userId]);

  const top = regions.find((stat) => stat.region?.kind === geo.RegionKind.ADMIN1) ?? regions[0];
  const lastPosition = toLatLng(location.data?.location?.position);
  const mapReady = !stats.isPending && !location.isPending;
  const start: { center: LatLng; zoom: number } = lastPosition
    ? { center: lastPosition, zoom: 13 }
    : top?.region?.center
      ? { center: toLatLng(top.region.center)!, zoom: 8 }
      : { center: DEFAULT_CENTER, zoom: 11 };

  return (
    <Page
      title={t("title")}
      back="/me"
      wide
      actions={
        <Button
          variant="secondary"
          size="sm"
          icon={<Gift className="size-4" />}
          onClick={() => setRecapOpen(true)}
        >
          {t("recap.open")}
        </Button>
      }
    >
      {/* Phones read top to bottom: the numbers, the map, then the rest. Wide screens keep the map beside it all. */}
      <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem] lg:items-start">
        <div className="lg:col-start-2">
          {stats.isPending ? (
            <Skeleton className="h-44" />
          ) : total === 0 ? (
            <div className="sticker rounded-card bg-surface p-5 text-center">
              <div className="animate-float text-6xl" aria-hidden>
                👣
              </div>
              <p className="mt-3 text-lg font-black">{t("empty.title")}</p>
              <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("empty.body")}</p>
              <Link
                to="/map"
                className="pressable sticker-sm mt-4 inline-flex h-11 items-center rounded-full bg-lime px-5 font-black text-[#17122b]"
              >
                {t("empty.action")}
              </Link>
            </div>
          ) : (
            <section className="sticker rounded-card bg-surface p-5" aria-label={t("hero.label")}>
              {levelUp && (
                <p className="mb-3 animate-pop-in rounded-2xl bg-sun px-4 py-2 text-center font-black text-[#17122b]">
                  🆙 {levelUp}
                </p>
              )}
              {top?.region && (
                <p className="text-2xl font-black leading-snug" data-testid="footprint-headline">
                  {t("hero.headline", {
                    region: regionName(top.region),
                    percent: formatPercent(top.percent),
                  })}
                </p>
              )}
              <div className="mt-4 flex items-center gap-4">
                <div className="sticker grid size-16 shrink-0 place-items-center rounded-2xl bg-lime text-[#17122b]">
                  <span className="text-center leading-none">
                    <span className="block text-[10px] font-black">Lv.</span>
                    <span className="block text-2xl font-black">{explorer.level}</span>
                  </span>
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-black">{t(`titles.${explorer.title}`)}</p>
                  <Meter
                    className="mt-1.5"
                    value={explorer.progress}
                    label={t("hero.levelProgress")}
                  />
                  <p className="mt-1 text-xs font-bold text-ink-soft">
                    {t("hero.toNext", { count: explorer.next - total, level: explorer.level + 1 })}
                  </p>
                </div>
              </div>
              <dl className="mt-4 grid grid-cols-3 gap-2 text-center">
                <Fact label={t("hero.cells")} value={total.toLocaleString()} />
                <Fact label={t("hero.countries")} value={String(stats.data?.countryCount ?? 0)} />
                <Fact
                  label={t("hero.area")}
                  value={`${(stats.data?.world?.exploredKm2 ?? 0).toFixed((stats.data?.world?.exploredKm2 ?? 0) < 100 ? 1 : 0)} km²`}
                />
              </dl>
            </section>
          )}
        </div>

        <div className="lg:sticky lg:top-20 lg:col-start-1 lg:row-span-2 lg:row-start-1">
          {mapReady ? (
            <FootprintMap
              center={start.center}
              zoom={start.zoom}
              fresh={recap.data?.newCells ?? []}
              className="sticker h-[24rem] rounded-card lg:h-[calc(100dvh-7.5rem)]"
            >
              <HereChip />
            </FootprintMap>
          ) : (
            <Skeleton className="h-[24rem] lg:h-[calc(100dvh-7.5rem)]" />
          )}
        </div>

        <div className="flex flex-col gap-5 lg:col-start-2">
          {recap.data && <RecapTeaser recap={recap.data} onOpen={() => setRecapOpen(true)} />}
          <RegionList stats={regions} coach onPick={setBoardRegion} />
          <Leaderboard
            regions={regions.flatMap((stat) => (stat.region ? [stat.region] : []))}
            regionId={boardRegion}
            onRegion={setBoardRegion}
            scope={boardScope}
            onScope={setBoardScope}
          />
          <p className="text-center text-xs text-ink-faint">{t("privacyNote")}</p>
          {regions.length > 0 && <GazetteerCredit className="text-center" />}
        </div>
      </div>
      <RecapDialog open={recapOpen} onOpenChange={setRecapOpen} />
    </Page>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl bg-surface-2 px-2 py-2.5">
      <dd className="truncate text-lg font-black tabular-nums">{value}</dd>
      <dt className="text-[11px] font-bold text-ink-faint">{label}</dt>
    </div>
  );
}

/** Names the prefecture under the middle of the map and how much of it is uncovered. */
function HereChip() {
  const map = useMap();
  const [center, setCenter] = useState(() => ({
    lat: Number(map.getCenter().lat.toFixed(1)),
    lng: Number(map.getCenter().lng.toFixed(1)),
  }));
  useEffect(() => {
    const update = () =>
      setCenter({
        lat: Number(map.getCenter().lat.toFixed(1)),
        lng: Number(map.getCenter().lng.toFixed(1)),
      });
    map.on("moveend", update);
    return () => {
      map.off("moveend", update);
    };
  }, [map]);
  const { data } = useQuery({
    queryKey: [...footprintKeys.all, "at", center.lat, center.lng],
    queryFn: () => footprintClient().getStats({ kind: geo.RegionKind.ADMIN1, at: center }),
    staleTime: 60_000,
    retry: false,
  });
  const here = data?.regions[0];
  if (!here?.region) return null;
  return (
    <p className="glass sticker-sm pointer-events-none absolute top-3 left-3 z-10 max-w-[calc(100%-1.5rem)] truncate rounded-full px-3 py-1 text-sm font-black">
      {regionName(here.region)} <span className="text-brand">{formatPercent(here.percent)}</span>
    </p>
  );
}

/** A friend's scratch map. What they do not show simply is not there. */
function FriendFootprints({ userId }: { userId: string }) {
  const { t } = useTranslation("footprints");
  const profile = useQuery({
    queryKey: ["identity", "profile", "id", userId],
    queryFn: () => profileClient().getProfile({ by: { case: "userId", value: userId } }),
    retry: false,
  });
  const stats = useQuery({
    queryKey: footprintKeys.stats(userId),
    queryFn: () => footprintClient().getStats({ userId }),
    retry: false,
  });
  // Cells are for friends; whether we may have them is the server's call.
  const probe = useQuery({
    queryKey: [...footprintKeys.all, "visible", userId],
    queryFn: () =>
      footprintClient().getCells({
        userId,
        bounds: { southWest: { lat: -85, lng: -180 }, northEast: { lat: 85, lng: 180 } },
        zoom: 1,
      }),
    retry: false,
  });

  const user = profile.data?.profile?.user;
  const name = user?.displayName ?? "";
  const regions = stats.data?.regions ?? [];
  const total = Number(stats.data?.totalCellCount ?? 0);
  const top = regions.find((stat) => stat.region?.kind === geo.RegionKind.ADMIN1) ?? regions[0];
  const pending = profile.isPending || stats.isPending || probe.isPending;
  // Three different things can go wrong, and only one of them is an answer:
  // the person does not exist, they do not show their footprints (the
  // server's refusal, shown as "kept to themselves"), or the request simply
  // failed, which is worth another try and must not read as either.
  const notFound = (error: unknown) =>
    errorReason(error) === "USER_NOT_FOUND" || (isApiError(error) && error.code === Code.NotFound);
  const refused = (error: unknown) =>
    errorReason(error) === "FOOTPRINTS_PRIVATE" ||
    (isApiError(error) && error.code === Code.PermissionDenied);
  const gone = notFound(profile.error) || notFound(stats.error);
  const withheld = !gone && refused(stats.error);
  const failed =
    !gone &&
    !withheld &&
    ((profile.isError && !notFound(profile.error)) ||
      stats.isError ||
      (probe.isError && !refused(probe.error) && !notFound(probe.error)));
  const retry = () => {
    for (const query of [profile, stats, probe]) if (query.isError) void query.refetch();
  };

  return (
    <Page title={name ? t("friend.title", { name }) : t("title")} back="/footprints" wide>
      {pending ? (
        <Skeleton className="h-64" />
      ) : gone ? (
        <Quiet emoji="🧭" title={t("friend.notFound")} />
      ) : withheld ? (
        <Quiet
          emoji="🤫"
          title={t("friend.privateTitle", { name })}
          body={t("friend.privateBody")}
        />
      ) : failed ? (
        <Quiet
          emoji="🌀"
          title={t("friend.failedTitle")}
          body={t("friend.failedBody")}
          action={
            <Button
              variant="secondary"
              loading={profile.isFetching || stats.isFetching || probe.isFetching}
              onClick={retry}
            >
              {t("friend.retry")}
            </Button>
          }
        />
      ) : (
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem] lg:items-start">
          {probe.isSuccess && (
            <div className="order-2 lg:sticky lg:top-20 lg:order-1">
              <FootprintMap
                userId={userId}
                center={toLatLng(top?.region?.center) ?? DEFAULT_CENTER}
                zoom={top ? 8 : 4}
                className="sticker h-[24rem] rounded-card lg:h-[calc(100dvh-7.5rem)]"
              />
            </div>
          )}
          <div className="order-1 flex flex-col gap-5 lg:order-2">
            <section className="sticker flex items-center gap-4 rounded-card bg-surface p-5">
              {user && (
                <Link to="/u/$handle" params={{ handle: user.handle }} aria-label={name}>
                  <Avatar user={user} size="lg" outlined />
                </Link>
              )}
              <div className="min-w-0 flex-1">
                <p className="truncate text-lg font-black">
                  {total > 0 ? t("friend.summary", { count: total }) : t("friend.none", { name })}
                </p>
                {total > 0 && (
                  <p className="text-sm font-bold text-ink-soft">
                    Lv.{explorerLevel(total).level} {t(`titles.${explorerLevel(total).title}`)} ·{" "}
                    {t("friend.countries", { count: stats.data?.countryCount ?? 0 })}
                  </p>
                )}
              </div>
            </section>
            <RegionList stats={regions} />
            {regions.length > 0 && <GazetteerCredit className="text-center" />}
          </div>
        </div>
      )}
    </Page>
  );
}

function Quiet({
  emoji,
  title,
  body,
  action,
}: {
  emoji: string;
  title: string;
  body?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-2 px-6 py-14 text-center">
      <div className="text-6xl" aria-hidden>
        {emoji}
      </div>
      <p className="mt-2 text-lg font-black">{title}</p>
      {body && <p className="max-w-xs text-sm leading-relaxed text-ink-soft">{body}</p>}
      {action && <div className="mt-3">{action}</div>}
    </div>
  );
}
