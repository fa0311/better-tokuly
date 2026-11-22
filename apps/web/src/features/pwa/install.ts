import { useSyncExternalStore } from "react";
import { create } from "zustand";
import { type ManualInstall, manualInstall } from "./platform";

/**
 * Getting Tokuly onto the home screen: the browser's install prompt where it
 * has one, the way to do it by hand where it does not, and whether the app
 * already runs installed. No UI in here.
 */

/** Chromium's "this site can be installed" event; `prompt()` opens the browser's own dialog, once. */
interface BeforeInstallPromptEvent extends Event {
  prompt(): Promise<unknown>;
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

interface InstallState {
  /** The browser handed over an install prompt that has not been used yet. */
  canPrompt: boolean;
  /** Installed during this visit: the tab is still a tab, but the app is on the home screen now. */
  justInstalled: boolean;
  /** The step-by-step guide for installing by hand is open. */
  guideOpen: boolean;
}

export const useInstall = create<InstallState>(() => ({
  canPrompt: false,
  justInstalled: false,
  guideOpen: false,
}));

let pendingPrompt: BeforeInstallPromptEvent | null = null;

// Listening starts when this module loads, not when a component mounts: the
// browser fires the event once, early, and does not repeat it for latecomers.
if (typeof window !== "undefined") {
  window.addEventListener("beforeinstallprompt", (event) => {
    // The app offers installing at a moment of its own choosing instead of the browser's banner.
    event.preventDefault();
    pendingPrompt = event as BeforeInstallPromptEvent;
    useInstall.setState({ canPrompt: true });
  });
  window.addEventListener("appinstalled", () => {
    pendingPrompt = null;
    useInstall.setState({ canPrompt: false, justInstalled: true, guideOpen: false });
  });
}

/** Opens the browser's install dialog. Only ever call this from a tap. */
export async function promptInstall(): Promise<"accepted" | "dismissed" | "unavailable"> {
  const prompt = pendingPrompt;
  if (!prompt) return "unavailable";
  // A prompt can be shown once; the browser sends a new event when it is ready to ask again.
  pendingPrompt = null;
  useInstall.setState({ canPrompt: false });
  try {
    await prompt.prompt();
    return (await prompt.userChoice).outcome;
  } catch {
    return "unavailable";
  }
}

export function openInstallGuide(): void {
  useInstall.setState({ guideOpen: true });
}

export function closeInstallGuide(): void {
  useInstall.setState({ guideOpen: false });
}

// ── Running installed ───────────────────────────────────────────────

/** Display modes of a window that belongs to the installed app. */
const INSTALLED_QUERY =
  "(display-mode: standalone), (display-mode: minimal-ui), (display-mode: window-controls-overlay)";

/** Whether this page runs as the installed app (opened from the home screen or in a window of its own). */
export function isStandalone(): boolean {
  if (typeof window === "undefined") return false;
  // `navigator.standalone` is what older iOS versions say instead of the media query.
  return (
    window.matchMedia(INSTALLED_QUERY).matches ||
    (navigator as Navigator & { standalone?: boolean }).standalone === true
  );
}

function subscribeToDisplayMode(onChange: () => void): () => void {
  // A tab becomes an app window when the browser moves it there after installing.
  const media = window.matchMedia(INSTALLED_QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}

/** `isStandalone()`, kept current. */
export function useIsStandalone(): boolean {
  return useSyncExternalStore(subscribeToDisplayMode, isStandalone, () => false);
}

// ── What to offer ───────────────────────────────────────────────────

/**
 * The way to install that fits this browser right now:
 * - `installed`: nothing to do, this is the installed app;
 * - `prompt`: one tap opens the browser's install dialog;
 * - anything else: by hand (see `ManualInstall`).
 */
export type InstallWay = "installed" | "prompt" | ManualInstall;

export function useInstallWay(): InstallWay {
  const standalone = useIsStandalone();
  const canPrompt = useInstall((state) => state.canPrompt);
  if (standalone) return "installed";
  if (canPrompt) return "prompt";
  if (typeof navigator === "undefined") return "none";
  return manualInstall(navigator.userAgent, navigator.maxTouchPoints);
}

/** Ways the guide has steps for. */
export function hasGuide(way: InstallWay): way is "ios-share" | "ios-browser" | "ios-in-app" {
  return way === "ios-share" || way === "ios-browser" || way === "ios-in-app";
}

// ── The offer, and being told "later" ───────────────────────────────

/** When the install banner was last waved away. */
const DISMISSED_KEY = "tokuly.pwa.installDismissedAt";
/** A dismissed banner stays away this long; the settings page offers installing all along. */
export const SNOOZE_MS = 30 * 24 * 60 * 60_000;

export function installOfferSnoozed(now: number = Date.now()): boolean {
  try {
    const at = Number(window.localStorage.getItem(DISMISSED_KEY) ?? 0);
    return Number.isFinite(at) && at > 0 && now - at < SNOOZE_MS;
  } catch {
    return false;
  }
}

export function snoozeInstallOffer(now: number = Date.now()): void {
  try {
    window.localStorage.setItem(DISMISSED_KEY, String(now));
  } catch {
    // Private browsing without storage: the banner comes back next time, which is all that is lost.
  }
}
