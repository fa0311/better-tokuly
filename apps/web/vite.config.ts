import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact, { reactCompilerPreset } from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig } from "vite";

/**
 * The browser only ever talks to the web origin. `/api/*` (Connect RPC),
 * `/media/*` (object storage), `/tiles/*` (map tiles) and `/map-assets/*`
 * (map fonts and icons) are forwarded by the server's route rules, both in
 * development and when the built server runs without an edge proxy in front
 * of it. Behind the production edge proxy the same paths are routed before
 * they reach this server.
 */
const api = process.env.TOKULY_DEV_API_URL ?? "http://127.0.0.1:8081";
const media = process.env.TOKULY_DEV_MEDIA_URL ?? "http://127.0.0.1:9000/tokuly-media";
const tiles = process.env.TOKULY_DEV_TILES_URL ?? "http://127.0.0.1:8090";
const mapAssets =
  process.env.TOKULY_DEV_MAP_ASSETS_URL ?? "https://protomaps.github.io/basemaps-assets";

export default defineConfig({
  resolve: { tsconfigPaths: true },
  // Module workers (the map's tile worker).
  worker: { format: "es" },
  plugins: [
    tailwindcss(),
    nitro({
      routeRules: {
        "/api/**": { proxy: `${api}/**` },
        "/media/**": { proxy: `${media}/**` },
        "/tiles/**": { proxy: `${tiles}/**` },
        "/map-assets/**": { proxy: `${mapAssets}/**` },
      },
    }),
    tanstackStart(),
    viteReact(),
    // React Compiler: memoizes components automatically, so the code has no manual useMemo/useCallback.
    babel({ presets: [reactCompilerPreset()] }),
  ],
});
