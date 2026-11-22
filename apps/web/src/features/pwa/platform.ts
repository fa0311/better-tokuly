/**
 * What can be told about the device from the browser's name: enough to say
 * how Tokuly gets onto its home screen when the browser has no install prompt
 * of its own. No React, no globals: everything is passed in.
 */

/**
 * How someone installs the app by hand in this browser.
 * - `ios-share`: Safari on an iPhone or iPad. Share → "Add to Home Screen".
 * - `ios-browser`: another browser on iOS (Chrome, Edge, Firefox). Their share menu has the same entry.
 * - `ios-in-app`: a page opened inside another app (LINE, Instagram, X…). It has to be opened in Safari first.
 * - `mac-dock`: Safari on a Mac. File → "Add to Dock".
 * - `menu`: a browser that installs from its own menu (Chrome, Edge, Samsung Internet, Firefox on Android).
 * - `none`: no way to install from here.
 */
export type ManualInstall = "ios-share" | "ios-browser" | "ios-in-app" | "mac-dock" | "menu" | "none";

/** Apps that show web pages in a browser of their own, where nothing can be installed. */
const IN_APP = /\bLine\/|Instagram|FBAN|FBAV|FB_IAB|Twitter|TikTok|musical_ly|Bytedance|Snapchat|KAKAOTALK|MicroMessenger|Discord|GSA\//i;
/** Browsers on iOS other than Safari (which only names itself "Safari"). */
const IOS_BROWSER = /CriOS|FxiOS|EdgiOS|OPiOS|OPT\/|DuckDuckGo|Brave/;

/** Whether this is an iPhone, iPod touch or iPad (an iPad calls itself a Mac, but has a touch screen). */
export function isIos(userAgent: string, maxTouchPoints: number): boolean {
  return /iPhone|iPad|iPod/.test(userAgent) || (userAgent.includes("Macintosh") && maxTouchPoints > 1);
}

export function manualInstall(userAgent: string, maxTouchPoints: number): ManualInstall {
  if (isIos(userAgent, maxTouchPoints)) {
    // Safari and real browsers end in "Safari/…"; a page inside another app usually does not.
    if (IN_APP.test(userAgent) || !/Safari\//.test(userAgent)) return "ios-in-app";
    return IOS_BROWSER.test(userAgent) ? "ios-browser" : "ios-share";
  }
  if (/Android/.test(userAgent)) {
    // "; wv)" marks a page inside another app on Android.
    if (IN_APP.test(userAgent) || /; wv\)/.test(userAgent)) return "none";
    return "menu";
  }
  if (/Chrome\/|Chromium\/|Edg\//.test(userAgent)) return "menu";
  if (userAgent.includes("Macintosh") && /Safari\//.test(userAgent)) return "mac-dock";
  return "none";
}
