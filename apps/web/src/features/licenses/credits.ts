/**
 * What the web app is made from that asks to be credited: the data on the map
 * and in place names, the fonts, the icons and the libraries in the bundle.
 * `/settings/licenses` lists it. A new dependency of the web app or of the
 * packages it bundles needs a line here: `credits.test.ts` checks that every
 * one is listed, with the licence its package declares.
 */

/** SPDX identifiers of the licences in use, and where each one can be read. */
export const LICENSES = {
  "ODbL-1.0": "https://opendatacommons.org/licenses/odbl/1-0/",
  "CC-BY-4.0": "https://creativecommons.org/licenses/by/4.0/",
  "OFL-1.1": "https://openfontlicense.org/open-font-license-official-text/",
  "Public Domain": "https://www.naturalearthdata.com/about/terms-of-use/",
  MIT: "https://opensource.org/license/mit",
  ISC: "https://opensource.org/license/isc-license-txt",
  "Apache-2.0": "https://www.apache.org/licenses/LICENSE-2.0",
  "BSD-3-Clause": "https://opensource.org/license/bsd-3-clause",
} as const;

export type License = keyof typeof LICENSES;

export interface Credit {
  name: string;
  url: string;
  /** The copyright line the licence asks to keep, when it names one. */
  holder?: string;
  licenses: License[];
  /** The npm package it comes in, for the check against the dependencies. */
  package?: string;
  /** What Tokuly uses it for: a key of `licenses:use`. */
  use?: string;
}

export const CREDIT_GROUPS: { id: "data" | "fonts" | "icons" | "libraries"; credits: Credit[] }[] =
  [
    {
      id: "data",
      credits: [
        {
          name: "OpenStreetMap",
          url: "https://www.openstreetmap.org/copyright",
          holder: "© OpenStreetMap contributors",
          licenses: ["ODbL-1.0"],
          use: "map",
        },
        {
          name: "Protomaps Basemaps",
          url: "https://github.com/protomaps/basemaps",
          holder: "© Protomaps",
          licenses: ["BSD-3-Clause"],
          package: "@protomaps/basemaps",
          use: "mapStyle",
        },
        {
          name: "GeoNames",
          url: "https://www.geonames.org/",
          holder: "© GeoNames",
          licenses: ["CC-BY-4.0"],
          use: "placeNames",
        },
        {
          name: "Natural Earth",
          url: "https://www.naturalearthdata.com/",
          licenses: ["Public Domain"],
          use: "regions",
        },
        {
          name: "timezone-boundary-builder",
          url: "https://github.com/evansiroky/timezone-boundary-builder",
          holder: "© OpenStreetMap contributors",
          licenses: ["ODbL-1.0"],
          use: "timeZones",
        },
      ],
    },
    {
      id: "fonts",
      credits: [
        {
          name: "M PLUS Rounded 1c",
          url: "https://github.com/coz-m/MPLUS_FONTS",
          holder: "© The Rounded M+ Project Authors",
          licenses: ["OFL-1.1"],
          package: "@fontsource/m-plus-rounded-1c",
          use: "appFont",
        },
        {
          name: "Noto Sans",
          url: "https://github.com/protomaps/basemaps-assets",
          holder: "© The Noto Project Authors",
          licenses: ["OFL-1.1"],
          use: "mapLabels",
        },
      ],
    },
    {
      id: "icons",
      credits: [
        {
          name: "Lucide",
          url: "https://lucide.dev/",
          holder: "© Lucide Icons and Contributors",
          licenses: ["ISC"],
          package: "lucide-react",
          use: "appIcons",
        },
        {
          name: "Tangrams icons",
          url: "https://github.com/tangrams/icons",
          holder: "© Mapzen",
          licenses: ["MIT"],
          use: "mapIcons",
        },
      ],
    },
    {
      id: "libraries",
      credits: [
        library("Base UI", "@base-ui/react", "https://base-ui.com/", "MIT"),
        library("Connect", "@connectrpc/connect", "https://connectrpc.com/", "Apache-2.0"),
        library(
          "Connect for the web",
          "@connectrpc/connect-web",
          "https://connectrpc.com/",
          "Apache-2.0",
        ),
        library("H3", "h3-js", "https://h3geo.org/", "Apache-2.0"),
        library("hls.js", "hls.js", "https://github.com/video-dev/hls.js", "Apache-2.0"),
        library("i18next", "i18next", "https://www.i18next.com/", "MIT"),
        library("LiveKit", "livekit-client", "https://livekit.io/", "Apache-2.0"),
        library("MapLibre GL JS", "maplibre-gl", "https://maplibre.org/", "BSD-3-Clause"),
        library(
          "MapLibre Style Spec",
          "@maplibre/maplibre-gl-style-spec",
          "https://maplibre.org/",
          "ISC",
        ),
        library("Motion", "motion", "https://motion.dev/", "MIT"),
        library("Nitro", "nitro", "https://nitro.build/", "MIT"),
        library(
          "Protobuf-ES",
          "@bufbuild/protobuf",
          "https://github.com/bufbuild/protobuf-es",
          "Apache-2.0",
          "BSD-3-Clause",
        ),
        library("node-qrcode", "qrcode", "https://github.com/soldair/node-qrcode", "MIT"),
        library("React", "react", "https://react.dev/", "MIT"),
        library("React DOM", "react-dom", "https://react.dev/", "MIT"),
        library("react-i18next", "react-i18next", "https://react.i18next.com/", "MIT"),
        library("SimpleWebAuthn", "@simplewebauthn/browser", "https://simplewebauthn.dev/", "MIT"),
        library("Sonner", "sonner", "https://sonner.emilkowal.ski/", "MIT"),
        library("TanStack Query", "@tanstack/react-query", "https://tanstack.com/query", "MIT"),
        library("TanStack Query Core", "@tanstack/query-core", "https://tanstack.com/query", "MIT"),
        library("TanStack Router", "@tanstack/react-router", "https://tanstack.com/router", "MIT"),
        library("TanStack Start", "@tanstack/react-start", "https://tanstack.com/start", "MIT"),
        library(
          "TanStack Virtual",
          "@tanstack/react-virtual",
          "https://tanstack.com/virtual",
          "MIT",
        ),
        library("ThumbHash", "thumbhash", "https://github.com/evanw/thumbhash", "MIT"),
        library(
          "class-variance-authority",
          "class-variance-authority",
          "https://cva.style/",
          "Apache-2.0",
        ),
        library("clsx", "clsx", "https://github.com/lukeed/clsx", "MIT"),
        library(
          "tailwind-merge",
          "tailwind-merge",
          "https://github.com/dcastil/tailwind-merge",
          "MIT",
        ),
        library("Zustand", "zustand", "https://github.com/pmndrs/zustand", "MIT"),
      ],
    },
  ];

function library(name: string, pkg: string, url: string, ...licenses: License[]): Credit {
  return { name, url, licenses, package: pkg };
}
