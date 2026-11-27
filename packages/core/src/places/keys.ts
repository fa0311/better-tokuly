import type { LatLng } from "./model";

/** Query keys of the places area. Everything about one place hangs under `placeKeys.place(id)`. */
export const placeKeys = {
  all: ["geo", "places"] as const,
  mine: ["geo", "places", "mine"] as const,
  saved: ["geo", "places", "saved"] as const,
  suggestions: ["geo", "places", "suggestions"] as const,
  place: (id: string) => ["geo", "places", "place", id] as const,
  search: (query: string, near: LatLng | null) =>
    ["geo", "places", "search", query, near?.lat ?? null, near?.lng ?? null] as const,
  reverse: (position: LatLng, language: string) =>
    [
      "geo",
      "reverse",
      Number(position.lat.toFixed(4)),
      Number(position.lng.toFixed(4)),
      language,
    ] as const,
};
