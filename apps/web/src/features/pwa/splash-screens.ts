// Written by tools/dev/icons.mjs together with the images in public/splash. Do not edit by hand.

/**
 * The launch image iOS shows while the installed app starts: one per iPhone
 * screen size and colour scheme, picked by `media`.
 */
export const SPLASH_SCREENS: readonly { media: string; href: string }[] = [
  // iPhone 17 Pro Max, 16 Pro Max
  { media: "(device-width: 440px) and (device-height: 956px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1320x2868-light.png" },
  // iPhone 17 Pro Max, 16 Pro Max
  { media: "(device-width: 440px) and (device-height: 956px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1320x2868-dark.png" },
  // iPhone Air
  { media: "(device-width: 420px) and (device-height: 912px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1260x2736-light.png" },
  // iPhone Air
  { media: "(device-width: 420px) and (device-height: 912px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1260x2736-dark.png" },
  // iPhone 17 Pro, 17, 16 Pro
  { media: "(device-width: 402px) and (device-height: 874px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1206x2622-light.png" },
  // iPhone 17 Pro, 17, 16 Pro
  { media: "(device-width: 402px) and (device-height: 874px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1206x2622-dark.png" },
  // iPhone 16 Plus, 15 Pro Max, 15 Plus, 14 Pro Max
  { media: "(device-width: 430px) and (device-height: 932px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1290x2796-light.png" },
  // iPhone 16 Plus, 15 Pro Max, 15 Plus, 14 Pro Max
  { media: "(device-width: 430px) and (device-height: 932px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1290x2796-dark.png" },
  // iPhone 16, 15 Pro, 15, 14 Pro
  { media: "(device-width: 393px) and (device-height: 852px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1179x2556-light.png" },
  // iPhone 16, 15 Pro, 15, 14 Pro
  { media: "(device-width: 393px) and (device-height: 852px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1179x2556-dark.png" },
  // iPhone 14 Plus, 13 Pro Max, 12 Pro Max
  { media: "(device-width: 428px) and (device-height: 926px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1284x2778-light.png" },
  // iPhone 14 Plus, 13 Pro Max, 12 Pro Max
  { media: "(device-width: 428px) and (device-height: 926px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1284x2778-dark.png" },
  // iPhone 16e, 14, 13 Pro, 13, 12 Pro, 12
  { media: "(device-width: 390px) and (device-height: 844px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1170x2532-light.png" },
  // iPhone 16e, 14, 13 Pro, 13, 12 Pro, 12
  { media: "(device-width: 390px) and (device-height: 844px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1170x2532-dark.png" },
  // iPhone 13 mini, 12 mini, 11 Pro, XS, X
  { media: "(device-width: 375px) and (device-height: 812px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1125x2436-light.png" },
  // iPhone 13 mini, 12 mini, 11 Pro, XS, X
  { media: "(device-width: 375px) and (device-height: 812px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1125x2436-dark.png" },
  // iPhone 11 Pro Max, XS Max
  { media: "(device-width: 414px) and (device-height: 896px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1242x2688-light.png" },
  // iPhone 11 Pro Max, XS Max
  { media: "(device-width: 414px) and (device-height: 896px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1242x2688-dark.png" },
  // iPhone 11, XR
  { media: "(device-width: 414px) and (device-height: 896px) and (-webkit-device-pixel-ratio: 2) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-828x1792-light.png" },
  // iPhone 11, XR
  { media: "(device-width: 414px) and (device-height: 896px) and (-webkit-device-pixel-ratio: 2) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-828x1792-dark.png" },
  // iPhone 8 Plus
  { media: "(device-width: 414px) and (device-height: 736px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-1242x2208-light.png" },
  // iPhone 8 Plus
  { media: "(device-width: 414px) and (device-height: 736px) and (-webkit-device-pixel-ratio: 3) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-1242x2208-dark.png" },
  // iPhone SE (2nd and 3rd generation), 8
  { media: "(device-width: 375px) and (device-height: 667px) and (-webkit-device-pixel-ratio: 2) and (orientation: portrait) and (prefers-color-scheme: light)", href: "/splash/iphone-750x1334-light.png" },
  // iPhone SE (2nd and 3rd generation), 8
  { media: "(device-width: 375px) and (device-height: 667px) and (-webkit-device-pixel-ratio: 2) and (orientation: portrait) and (prefers-color-scheme: dark)", href: "/splash/iphone-750x1334-dark.png" },
];
