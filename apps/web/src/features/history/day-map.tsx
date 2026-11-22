import { Pause, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Slider } from "#/components/ui/slider";
import { cameraMs, FitTo, GeoLayer, MapFrame, Pin, useIsDark, useMap } from "#/features/map";
import { DEFAULT_CENTER, type LatLng } from "#/features/places";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { positionAt, type TrackPoint } from "./timeline";

/** How long a whole day takes to replay at normal speed. */
const REPLAY_MS = 12_000;
const SCRUB_STEPS = 1000;

/** A stop of the day to mark on the map. */
export interface DayStop {
  id: string;
  position: LatLng;
  /** Its number in the day's order. */
  order: number;
  emoji: string;
  name: string;
}

/** A line through the points as GeoJSON; nothing at all for fewer than two. */
const line = (points: LatLng[]) => ({
  type: "FeatureCollection" as const,
  features:
    points.length < 2
      ? []
      : [
          {
            type: "Feature" as const,
            properties: {},
            geometry: {
              type: "LineString" as const,
              coordinates: points.map((point) => [point.lng, point.lat] as [number, number]),
            },
          },
        ],
});

/**
 * The map of a day: the route, the stops in order, and — when the route
 * carries times (`track`) — a replay with a scrubber. Without times
 * (`path` only) the line is simply drawn; without either, the stops alone.
 */
export function DayMap({
  date,
  track,
  path,
  stops,
  focus,
  onStop,
  className,
}: {
  date: string;
  track: TrackPoint[];
  path: LatLng[];
  stops: DayStop[];
  /** The stop to bring into view (picked in the timeline). */
  focus: { id: string; nonce: number } | null;
  onStop(id: string): void;
  className?: string;
}) {
  const points: LatLng[] = track.length > 1 ? track : path;
  const all = [...points, ...stops.map((stop) => stop.position)];
  const start = all[0] ?? DEFAULT_CENTER;
  return (
    <MapFrame center={start} zoom={14} className={className}>
      <FitTo points={all} fitKey={`${date}:${all.length}`} padding={64} />
      <Route track={track} path={path} />
      {stops.map((stop) => (
        <Pin key={stop.id} position={stop.position} zIndex={3}>
          <button
            type="button"
            onClick={() => onStop(stop.id)}
            aria-label={`${stop.order}. ${stop.name}`}
            className="group flex flex-col items-center outline-none"
          >
            <span className="sticker relative grid size-11 place-items-center rounded-2xl bg-surface text-xl transition-transform duration-200 ease-bounce group-hover:scale-110">
              {stop.emoji}
              <span className="absolute -top-2 -right-2 grid size-5 place-items-center rounded-full border-2 border-ink bg-lime text-[10px] font-black text-[#17122b]">
                {stop.order}
              </span>
            </span>
            <span className="h-2 w-[3px] bg-ink" aria-hidden />
          </button>
        </Pin>
      ))}
      <Focus stops={stops} focus={focus} />
    </MapFrame>
  );
}

function Focus({
  stops,
  focus,
}: {
  stops: DayStop[];
  focus: { id: string; nonce: number } | null;
}) {
  const map = useMap();
  const latest = useRef(stops);
  latest.current = stops;
  useEffect(() => {
    const stop = focus && latest.current.find((candidate) => candidate.id === focus.id);
    if (stop)
      map.easeTo({
        center: [stop.position.lng, stop.position.lat],
        zoom: Math.max(map.getZoom(), 15),
        duration: cameraMs(600),
      });
  }, [map, focus]);
  return null;
}

/** The route line and, when it has times, the replay controls. */
function Route({ track, path }: { track: TrackPoint[]; path: LatLng[] }) {
  const { t, i18n } = useTranslation("history");
  const dark = useIsDark();
  const me = useMe();
  const timed = track.length > 1;
  const [progress, setProgress] = useState(SCRUB_STEPS);
  const [playing, setPlaying] = useState(false);
  const progressRef = useRef(progress);
  progressRef.current = progress;

  const first = track[0]?.at ?? 0;
  const last = track.at(-1)?.at ?? 0;
  const at = first + ((last - first) * progress) / SCRUB_STEPS;
  const reached = timed ? positionAt(track, at) : null;

  useEffect(() => {
    if (!playing) return;
    // Motion turned off: the replay still runs, in a handful of jumps instead of a glide.
    const reduced =
      window.matchMedia("(prefers-reduced-motion: reduce)").matches || dopamineFactor() <= 0;
    const from = progressRef.current >= SCRUB_STEPS ? 0 : progressRef.current;
    const began = performance.now();
    let frame = 0;
    let timer: ReturnType<typeof setInterval> | undefined;
    const advance = () => {
      const next = Math.min(
        SCRUB_STEPS,
        from + ((performance.now() - began) / REPLAY_MS) * SCRUB_STEPS,
      );
      setProgress(next);
      if (next >= SCRUB_STEPS) setPlaying(false);
      return next < SCRUB_STEPS;
    };
    if (reduced) timer = setInterval(advance, 1000);
    else {
      const step = () => {
        if (advance()) frame = requestAnimationFrame(step);
      };
      frame = requestAnimationFrame(step);
    }
    // A replay nobody is watching stops where it is (the cleanup below ends the frames and the interval).
    const onHidden = () => {
      if (document.visibilityState === "hidden") setPlaying(false);
    };
    document.addEventListener("visibilitychange", onHidden);
    return () => {
      cancelAnimationFrame(frame);
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onHidden);
    };
  }, [playing]);

  const colour = dark
    ? { rest: "#6d6594", done: "#c6f24e", casing: "#0d0b14" }
    : { rest: "#b3a7dc", done: "#7c5cff", casing: "#ffffff" };
  const whole: LatLng[] = timed ? track : path;
  const done: LatLng[] = timed && reached ? [...track.slice(0, reached.index + 1), reached] : whole;

  return (
    <>
      <GeoLayer
        id="day-route"
        data={line(whole)}
        layers={[
          {
            id: "day-route-casing",
            type: "line",
            layout: { "line-cap": "round", "line-join": "round" },
            paint: { "line-color": colour.casing, "line-width": 8, "line-opacity": 0.9 },
          },
          {
            id: "day-route-rest",
            type: "line",
            layout: { "line-cap": "round", "line-join": "round" },
            paint: { "line-color": colour.rest, "line-width": 4, "line-dasharray": [1, 1.6] },
          },
        ]}
      />
      <GeoLayer
        id="day-route-done"
        data={line(done)}
        layers={[
          {
            id: "day-route-done-line",
            type: "line",
            layout: { "line-cap": "round", "line-join": "round" },
            paint: { "line-color": colour.done, "line-width": 5 },
          },
        ]}
      />
      {reached && me?.user && (
        <Pin position={reached} anchor="center" zIndex={6}>
          <span className="sticker-sm block rounded-xl bg-brand p-[2px]">
            <Avatar user={me.user} size="sm" className="block" />
          </span>
        </Pin>
      )}
      {whole.length > 1 && (
        <div
          className="glass sticker-sm absolute inset-x-3 bottom-3 z-10 flex items-center gap-2 rounded-full py-1 pr-4 pl-1.5"
          data-testid="day-route"
          data-points={whole.length}
          data-timed={timed}
        >
          {timed ? (
            <>
              <button
                type="button"
                aria-label={playing ? t("replay.pause") : t("replay.play")}
                onClick={() => setPlaying((current) => !current)}
                className={cn(
                  "pressable grid size-9 shrink-0 place-items-center rounded-full border-2 border-ink",
                  playing ? "bg-surface" : "bg-lime text-[#17122b]",
                )}
              >
                {playing ? <Pause className="size-4" /> : <Play className="size-4" />}
              </button>
              <Slider
                className="min-w-0 flex-1"
                value={progress}
                max={SCRUB_STEPS}
                label={t("replay.scrub")}
                valueText={new Date(at).toLocaleTimeString(i18n.language, {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
                onChange={(value) => {
                  setPlaying(false);
                  setProgress(value);
                }}
              />
              <span
                className="w-12 shrink-0 text-right text-sm font-black tabular-nums"
                data-testid="replay-time"
              >
                {new Date(at).toLocaleTimeString(i18n.language, {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </span>
            </>
          ) : (
            <p className="py-1.5 pl-3 text-xs font-bold text-ink-soft">{t("replay.pathOnly")}</p>
          )}
        </div>
      )}
    </>
  );
}
