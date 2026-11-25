import { TILE_ZOOMS, type TileConfig } from "@tokuly/core/map";

/**
 * The Tokuly server of this build (`EXPO_PUBLIC_TOKULY_URL`, inlined at build
 * time): one origin that serves the API, the realtime fallback, media, tiles
 * and the web app. In development it is the web dev server, which forwards
 * every path to the dev API (`adb reverse tcp:3000 tcp:3000` on Android).
 */
export const SERVER_URL = (process.env.EXPO_PUBLIC_TOKULY_URL ?? "http://localhost:3000").replace(
  /\/+$/,
  "",
);

export const API_URL = `${SERVER_URL}/api`;

/** The web app, for what is not native yet. */
export const WEB_URL = SERVER_URL;

export const TILE_CONFIG: TileConfig = {
  tilesUrl: `${SERVER_URL}/tiles`,
  assetsUrl: `${SERVER_URL}/map-assets`,
  ...TILE_ZOOMS,
};

/** A link the server shares (an invite, a profile) in full. */
export function webLink(path: string): string {
  return `${WEB_URL}${path.startsWith("/") ? path : `/${path}`}`;
}
