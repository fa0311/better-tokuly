import { Map as MapLibreMap } from "maplibre-gl";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { useIsDark } from "./map-view";
import { buildMapStyle } from "./tiles";

/**
 * Pictures of the map, drawn one at a time by a single hidden map and kept
 * as images. A page full of previews (a visit history, a chat with several
 * locations) therefore costs one WebGL context, not one per preview —
 * browsers only allow a handful.
 */
const WIDTH = 480;
const HEIGHT = 270;
const CACHE_LIMIT = 80;
const cache = new Map<string, Promise<string>>();
let painter: { map: MapLibreMap; style: string } | null = null;
let queue: Promise<unknown> = Promise.resolve();

function paint(
  lat: number,
  lng: number,
  zoom: number,
  dark: boolean,
  language: string,
): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const style = `${dark}:${language}`;
    if (!painter) {
      const container = document.createElement("div");
      container.style.cssText = `position:fixed;left:-10000px;top:0;width:${WIDTH}px;height:${HEIGHT}px;pointer-events:none;visibility:hidden`;
      container.setAttribute("aria-hidden", "true");
      document.body.append(container);
      painter = {
        style,
        map: new MapLibreMap({
          container,
          style: buildMapStyle("tokuly", dark, language),
          center: [lng, lat],
          zoom,
          interactive: false,
          attributionControl: false,
          fadeDuration: 0,
          pixelRatio: Math.min(2, window.devicePixelRatio || 1),
          // The picture is read back from the canvas after the frame was drawn.
          canvasContextAttributes: { preserveDrawingBuffer: true },
        }),
      };
    } else {
      if (painter.style !== style) {
        painter.style = style;
        painter.map.setStyle(buildMapStyle("tokuly", dark, language));
      }
      painter.map.jumpTo({ center: [lng, lat], zoom });
      // The same view as last time draws nothing by itself, and nothing drawn means no `idle`.
      painter.map.triggerRepaint();
    }
    const { map } = painter;
    const timeout = setTimeout(() => reject(new Error("the map preview took too long")), 15_000);
    map.once("idle", () => {
      clearTimeout(timeout);
      try {
        resolve(map.getCanvas().toDataURL("image/webp", 0.8));
      } catch (error) {
        reject(error);
      }
    });
    map.once("error", () => undefined);
  });
}

function mapPicture(
  lat: number,
  lng: number,
  zoom: number,
  dark: boolean,
  language: string,
): Promise<string> {
  const key = `${lat.toFixed(5)},${lng.toFixed(5)},${zoom},${dark},${language}`;
  const cached = cache.get(key);
  if (cached) return cached;
  const picture = queue.then(() => paint(lat, lng, zoom, dark, language));
  queue = picture.catch(() => undefined);
  cache.set(key, picture);
  picture.catch(() => cache.delete(key));
  if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value as string);
  return picture;
}

export interface StaticMapPreviewProps {
  lat: number;
  lng: number;
  /** Default 15 (a neighbourhood). */
  zoom?: number;
  /** What sits on the spot. Default: a 📍 sticker. Pass `null` for no pin. */
  pin?: ReactNode;
  /** Classes for the box. Give it a size (default: full width, 16:9, rounded). */
  className?: string;
  /** Read aloud instead of the picture, e.g. the place name. */
  label?: string;
}

/**
 * A picture of the map around a point, with a pin on it: for place cards,
 * history entries, location messages and place tags. It is an image, not a
 * live map — cheap enough for lists. It is drawn when it scrolls into view
 * and follows the light / dark theme.
 */
export function StaticMapPreview({
  lat,
  lng,
  zoom = 15,
  pin,
  className,
  label,
}: StaticMapPreviewProps) {
  const { i18n } = useTranslation();
  const dark = useIsDark();
  const language = i18n.language.startsWith("ja") ? "ja" : "en";
  const box = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const [picture, setPicture] = useState<{ key: string; url: string } | null>(null);
  const key = `${lat},${lng},${zoom},${dark},${language}`;

  useEffect(() => {
    if (!box.current || visible) return;
    const observer = new IntersectionObserver(
      (entries) => entries.some((entry) => entry.isIntersecting) && setVisible(true),
      { rootMargin: "200px" },
    );
    observer.observe(box.current);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
    mapPicture(lat, lng, zoom, dark, language).then(
      (url) => !cancelled && setPicture({ key, url }),
      // No WebGL, or tiles out of reach: the pin on the plain background still says "a place".
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [visible, lat, lng, zoom, dark, language, key]);

  return (
    <div
      ref={box}
      role="img"
      aria-label={label ?? `${lat.toFixed(4)}, ${lng.toFixed(4)}`}
      className={cn(
        "relative aspect-video w-full overflow-hidden rounded-2xl bg-surface-3",
        className,
      )}
      data-ready={picture?.key === key || undefined}
    >
      {picture && (
        <img
          src={picture.url}
          alt=""
          className={cn(
            "size-full object-cover transition-opacity duration-300",
            picture.key !== key && "opacity-60",
          )}
          draggable={false}
        />
      )}
      {pin !== null && (
        <span
          className="absolute top-1/2 left-1/2 flex -translate-x-1/2 -translate-y-full flex-col items-center"
          aria-hidden
        >
          {pin ?? (
            <span className="sticker-sm grid size-9 place-items-center rounded-xl bg-surface text-lg">
              📍
            </span>
          )}
          <span className="h-2 w-[3px] bg-ink" />
        </span>
      )}
    </div>
  );
}
