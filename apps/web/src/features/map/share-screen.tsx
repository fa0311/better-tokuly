import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type geo, isNotFound, toMillis } from "@tokuly/api";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LogoMark } from "#/components/ui/logo";
import { PaneSpinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { compactAge, compactDuration, kmh } from "#/lib/format";
import { useSessionStatus } from "#/stores/session";
import { locationClient, useNow } from "./data";
import { MapPin, MapView, useMap } from "./map-view";
import { MOVING_SPEED_MPS, STALE_AFTER_MS } from "./model";
import { ACTIVITY_EMOJI } from "./pins";

/** Keeps the shared pin in view as it moves. */
function Follow({ lat, lng }: { lat: number; lng: number }) {
  const map = useMap();
  useEffect(() => {
    map.easeTo({ center: [lng, lat], zoom: Math.max(map.getZoom(), 15), duration: 900 });
  }, [map, lat, lng]);
  return null;
}

/**
 * `/share/<token>`: someone's live position for anyone who has the link — no
 * account, no app. The pin moves as they do, and the page says so when the
 * share has ended.
 */
export function ShareScreen({ token }: { token: string }) {
  const signedIn = useSessionStatus() === "signedIn";
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const now = useNow(10_000);
  const queryKey = ["geo", "liveShare", token];
  const share = useQuery({
    queryKey,
    queryFn: () => locationClient().getLiveShare({ token }),
    retry: (count, error) => !isNotFound(error) && count < 2,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const [live, setLive] = useState<geo.FriendLocation | null>(null);
  const [ended, setEnded] = useState(false);
  const found = share.isSuccess;

  // The moving pin: a stream that stays open for as long as the share runs.
  useEffect(() => {
    if (!found) return;
    // Once a share is over nothing of it stays behind: not the last position, not the cached answer.
    const end = () => {
      setEnded(true);
      setLive(null);
      queryClient.removeQueries({ queryKey: ["geo", "liveShare", token], exact: true });
    };
    const abort = new AbortController();
    let retry: ReturnType<typeof setTimeout> | undefined;
    const watch = async () => {
      try {
        for await (const update of locationClient().watchLiveShare(
          { token },
          { signal: abort.signal },
        )) {
          if (update.location) setLive(update.location);
          if (update.ended) {
            end();
            return;
          }
        }
        // The server closed the stream without saying "ended" (a restart): pick it up again.
        if (!abort.signal.aborted) retry = setTimeout(watch, 3000);
      } catch (error) {
        if (abort.signal.aborted) return;
        if (isNotFound(error)) end();
        else retry = setTimeout(watch, 5000);
      }
    };
    void watch();
    return () => {
      abort.abort();
      clearTimeout(retry);
    };
  }, [found, token, queryClient]);

  // The link's end time by this device's clock is only a reason to ask the server again.
  const expiresAt = toMillis(share.data?.expiresAt);
  const expired = expiresAt > 0 && expiresAt <= now;
  const refetch = share.refetch;
  useEffect(() => {
    if (expired) void refetch();
  }, [expired, refetch]);

  const over = ended || (share.isError && isNotFound(share.error));
  if (!over && share.isPending) {
    return (
      <div className="grid h-dvh place-items-center bg-bg">
        <PaneSpinner />
      </div>
    );
  }
  if (!over && share.isError && !share.data) {
    // Not "ended": the page could not ask. Saying the share is over would be a guess.
    return (
      <main className="grid h-dvh place-items-center bg-bg p-6">
        <div
          className="sticker-lg flex max-w-sm flex-col items-center gap-3 rounded-[2rem] bg-surface p-8 text-center"
          data-testid="share-error"
          role="alert"
        >
          <span className="text-6xl" aria-hidden>
            📡
          </span>
          <h1 className="text-2xl font-black">{t("public.errorTitle")}</h1>
          <p className="text-sm leading-relaxed text-ink-soft">{errorMessage(share.error)}</p>
          <Button onClick={() => void share.refetch()} loading={share.isFetching}>
            {t("public.retry")}
          </Button>
        </div>
      </main>
    );
  }
  if (over) {
    return (
      <main className="grid h-dvh place-items-center bg-bg p-6">
        <div
          className="sticker-lg flex max-w-sm flex-col items-center gap-3 rounded-[2rem] bg-surface p-8 text-center"
          data-testid="share-ended"
        >
          <span className="animate-float text-6xl" aria-hidden>
            👋
          </span>
          <h1 className="text-2xl font-black">{t("public.endedTitle")}</h1>
          <p className="text-sm leading-relaxed text-ink-soft">{t("public.endedBody")}</p>
          <Link
            to="/"
            className="pressable sticker mt-2 flex h-12 items-center gap-2 rounded-full bg-brand px-6 font-black text-on-brand"
          >
            <LogoMark className="size-6" /> {t("public.cta")}
          </Link>
        </div>
      </main>
    );
  }

  const user = share.data?.user;
  const location = live ?? share.data?.location ?? null;
  const position = location?.position;
  const recordedAt = toMillis(location?.recordedAt);
  const stale = !location || location.sharingPaused || now - recordedAt > STALE_AFTER_MS;
  const moving = Boolean(location) && !stale && (location?.speedMps ?? 0) >= MOVING_SPEED_MPS;

  return (
    <main className="relative h-dvh overflow-hidden bg-bg">
      <MapView
        initialCenter={position ? [position.lng, position.lat] : undefined}
        initialZoom={position ? 15 : 10}
      >
        {position && user && (
          <>
            <Follow lat={position.lat} lng={position.lng} />
            <MapPin lat={position.lat} lng={position.lng} glide={moving ? "steady" : "settle"}>
              <div
                className={
                  stale
                    ? "flex flex-col items-center opacity-70 saturate-50"
                    : "flex flex-col items-center"
                }
                data-testid="share-pin"
                data-lat={position.lat.toFixed(5)}
                data-lng={position.lng.toFixed(5)}
              >
                <span className="relative block">
                  {!stale && (
                    <span
                      className="absolute inset-0 animate-pulse-ring rounded-[1.25rem] bg-brand"
                      aria-hidden
                    />
                  )}
                  <span className="sticker relative block rounded-[1.25rem] bg-surface p-[3px]">
                    <Avatar user={user} size="lg" className="block [&>span]:rounded-[1rem]" />
                  </span>
                </span>
                <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
                <span className="rounded-full bg-ink px-2.5 py-0.5 text-[11px] font-black whitespace-nowrap text-surface">
                  {moving
                    ? `${ACTIVITY_EMOJI[location?.activity ?? 0] ?? ""} ${kmh(location?.speedMps ?? 0)} km/h`.trim()
                    : compactAge(recordedAt, now)}
                </span>
              </div>
            </MapPin>
          </>
        )}
      </MapView>

      <header className="pointer-events-none absolute inset-x-0 top-0 z-20 flex justify-center p-4 pt-safe">
        <div
          className="glass sticker pointer-events-auto flex w-full max-w-md items-center gap-3 rounded-3xl p-3"
          data-testid="share-card"
        >
          <Avatar user={user} size="md" outlined />
          <div className="min-w-0 flex-1">
            <h1 className="truncate font-black">
              {t("public.title", { name: user?.displayName })}
            </h1>
            <p className="truncate text-xs font-bold text-ink-soft">
              {share.data?.label ? `${share.data.label} · ` : ""}
              {expiresAt > 0 && t("public.endsIn", { time: compactDuration(expiresAt - now) })}
            </p>
            <p className="truncate text-xs font-bold text-ink-soft" data-testid="share-status">
              {!position
                ? t("public.noPosition")
                : location?.sharingPaused
                  ? t("public.paused")
                  : stale
                    ? t("public.stale", { age: compactAge(recordedAt, now) })
                    : [location?.locality, t("public.live")].filter(Boolean).join(" · ")}
            </p>
          </div>
        </div>
      </header>

      <footer className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex justify-center p-4 pb-safe">
        {/* A visitor who makes an account comes back to the share they were looking at. */}
        <Link
          to={signedIn ? "/map" : "/signup"}
          search={signedIn ? undefined : { redirect: `/share/${token}` }}
          className="glass sticker-sm pressable pointer-events-auto mb-4 flex h-11 items-center gap-2 rounded-full px-4 text-sm font-black"
        >
          <LogoMark className="size-6" /> {t("public.cta")}
        </Link>
      </footer>
    </main>
  );
}
