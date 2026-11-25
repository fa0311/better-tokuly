import { isBlurred } from "@tokuly/core/map";
import { useLocations } from "#/stores/locations";
import { useMap } from "./map-view";

function reducedMotion(): boolean {
  return (
    window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
    document.documentElement.dataset.dopa === "0"
  );
}

/**
 * Where the camera was last sent. Opening a sheet changes the map's padding
 * a moment after the tap that flew somewhere; whoever applies the padding
 * finishes that flight instead of cutting it short.
 */
let aim: { lng: number; lat: number; zoom: number; at: number } | null = null;

export function recentAim(): { lng: number; lat: number; zoom: number } | null {
  return aim && performance.now() - aim.at < 1500 ? aim : null;
}

/**
 * Flies the map to a point. It zooms in to `minZoom` when farther out, and
 * out to `maxZoom` when closer in (to show a whole area), otherwise the zoom
 * stays as the person left it.
 */
export function useFlyTo(): (lng: number, lat: number, minZoom?: number, maxZoom?: number) => void {
  const map = useMap();
  return (lng, lat, minZoom = 15, maxZoom = 22) => {
    const zoom = Math.min(Math.max(map.getZoom(), minZoom), Math.max(minZoom, maxZoom));
    aim = { lng, lat, zoom, at: performance.now() };
    if (reducedMotion()) map.jumpTo({ center: [lng, lat], zoom });
    else map.flyTo({ center: [lng, lat], zoom, speed: 1.6, curve: 1.4, essential: true });
  };
}

/** Flies to a friend's pin, if they have one. A blurred pin is shown with its whole area. */
export function useFlyToUser(): (userId: string) => void {
  const flyTo = useFlyTo();
  return (userId) => {
    const location = useLocations.getState().friends[userId];
    if (!location?.position) return;
    // A blurred pin stands for an area: show the whole of it, whatever accuracy the server names.
    if (isBlurred(location)) flyTo(location.position.lng, location.position.lat, 11, 12.5);
    else flyTo(location.position.lng, location.position.lat, 15);
  };
}
