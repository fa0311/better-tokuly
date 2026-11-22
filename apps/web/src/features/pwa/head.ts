import { SPLASH_SCREENS } from "./splash-screens";

/**
 * What the document's <head> needs to be installable and to look like an app
 * once it is: spread into the root route's `meta` and `links`. Plain data,
 * safe to import on the server.
 */

/** The page background (`--bg` in src/styles.css) of each theme: the colour of the bars around the page. */
const BACKGROUND = { light: "#f4f1ff", dark: "#0d0b14" } as const;

export const PWA_META = [
  // One colour per colour scheme. The router keeps a single <meta> per `name`,
  // so the second is spelled differently; browsers read the name without regard to case.
  { name: "theme-color", media: "(prefers-color-scheme: light)", content: BACKGROUND.light },
  { name: "Theme-Color", media: "(prefers-color-scheme: dark)", content: BACKGROUND.dark },
  { name: "application-name", content: "Tokuly" },
  // Opened from the home screen, the page gets a window of its own (iOS only reads the apple- tags).
  { name: "mobile-web-app-capable", content: "yes" },
  { name: "apple-mobile-web-app-capable", content: "yes" },
  { name: "apple-mobile-web-app-title", content: "Tokuly" },
  // The status bar keeps its own strip in the theme colour; the page starts below it.
  { name: "apple-mobile-web-app-status-bar-style", content: "default" },
];

export const PWA_LINKS = [
  // The last icon a browser understands wins: the SVG where it can, a PNG elsewhere.
  { rel: "icon", type: "image/png", sizes: "16x16", href: "/icons/favicon-16.png" },
  { rel: "icon", type: "image/png", sizes: "32x32", href: "/icons/favicon-32.png" },
  { rel: "icon", type: "image/svg+xml", href: "/icon.svg" },
  { rel: "apple-touch-icon", sizes: "180x180", href: "/icons/apple-touch-icon.png" },
  { rel: "manifest", href: "/manifest.webmanifest" },
  ...SPLASH_SCREENS.map(({ media, href }) => ({ rel: "apple-touch-startup-image", media, href })),
];
