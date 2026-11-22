/** The time zone this browser runs in. */
export function deviceTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** Every IANA time zone the browser knows, with UTC. */
export function allTimeZones(): string[] {
  const zones =
    typeof Intl.supportedValuesOf === "function"
      ? Intl.supportedValuesOf("timeZone")
      : [deviceTimeZone()];
  return zones.includes("UTC") ? zones : ["UTC", ...zones];
}

/** "Asia/Ho_Chi_Minh" → "Asia/Ho Chi Minh". */
export function zoneLabel(zone: string): string {
  return zone.replaceAll("_", " ");
}

/** The zone's current offset as "UTC+9" / "UTC-3:30"; empty for a zone the browser does not know. */
export function zoneOffset(zone: string, at = new Date()): string {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      timeZoneName: "shortOffset",
    }).formatToParts(at);
    const name = parts.find((part) => part.type === "timeZoneName")?.value ?? "";
    // "GMT+9" / "GMT-3:30"; UTC itself comes as "GMT" or "GMT+0" depending on the browser.
    const offset = name.replace("GMT", "");
    return offset === "" || /^[+-]0$/.test(offset) ? "UTC" : `UTC${offset}`;
  } catch {
    return "";
  }
}

/** Zones whose name contains every word of the query (case and `_` insensitive). */
export function searchTimeZones(zones: string[], query: string): string[] {
  const words = query
    .toLowerCase()
    .split(/[\s_/]+/)
    .filter(Boolean);
  if (words.length === 0) return zones;
  return zones.filter((zone) => {
    const name = zone.toLowerCase().replaceAll("_", " ");
    return words.every((word) => name.includes(word));
  });
}
