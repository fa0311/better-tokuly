import i18n from "#/lib/i18n";

/** "NOW" within 10 s like the original marker pill, then a compact age. */
export function compactAge(fromMs: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.floor((nowMs - fromMs) / 1000));
  if (seconds <= 10) return i18n.t("time.now");
  if (seconds < 60) return i18n.t("time.justNow");
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return i18n.t("time.minutesAgo", { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return i18n.t("time.hoursAgo", { count: hours });
  return i18n.t("time.daysAgo", { count: Math.floor(hours / 24) });
}

/** A duration like "45分" / "3時間" / "2日" (ja) or "45m" / "3h" / "2d" (en). */
export function compactDuration(ms: number): string {
  const ja = i18n.language.startsWith("ja");
  const minutes = Math.max(1, Math.floor(ms / 60_000));
  if (minutes < 60) return ja ? `${minutes}分` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return ja ? `${hours}時間` : `${hours}h`;
  const days = Math.floor(hours / 24);
  return ja ? `${days}日` : `${days}d`;
}

/** Metres per second → whole km/h. */
export function kmh(speedMps: number): number {
  return Math.round(speedMps * 3.6);
}

/** A distance for humans: "850 m", "8.5 km", "8,546 km". */
export function formatDistance(meters: number, unit: "km" | "mi" = "km"): string {
  if (unit === "mi") {
    const miles = meters / 1609.344;
    return miles < 0.1
      ? `${Math.round(meters * 3.28084)} ft`
      : `${miles < 10 ? miles.toFixed(1) : Math.round(miles).toLocaleString()} mi`;
  }
  if (meters < 1000) return `${Math.round(meters / 10) * 10} m`;
  const km = meters / 1000;
  return `${km < 10 ? km.toFixed(1) : Math.round(km).toLocaleString()} km`;
}

/** Great-circle distance in metres between two coordinates. */
export function distanceMeters(
  a: { lat: number; lng: number },
  b: { lat: number; lng: number },
): number {
  const R = 6_371_008.8;
  const toRad = (deg: number) => (deg * Math.PI) / 180;
  const dLat = toRad(b.lat - a.lat);
  const dLng = toRad(b.lng - a.lng);
  const h =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(toRad(a.lat)) * Math.cos(toRad(b.lat)) * Math.sin(dLng / 2) ** 2;
  return 2 * R * Math.asin(Math.min(1, Math.sqrt(h)));
}
