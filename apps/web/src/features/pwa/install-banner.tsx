import { useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import {
  hasGuide,
  installOfferSnoozed,
  openInstallGuide,
  promptInstall,
  snoozeInstallOffer,
  useInstallWay,
} from "./install";
import { useOnline } from "./online";

/**
 * Pages where the offer may appear: the calm top-level ones. Never the map
 * (its own cards live where the banner would go), a composer, a player or a
 * page in the middle of a task, and not the settings page that already
 * offers installing.
 */
const CALM_PAGES = new Set([
  "/me",
  "/friends",
  "/notifications",
  "/settings",
  "/stamps",
  "/wallet",
  "/announcements",
]);
/** The app gets this long to settle after it starts before the offer may slide in. */
const SETTLE_MS = 2500;

/**
 * The floating offer to put Tokuly on the home screen. It is an overlay above
 * the tab bar, so nothing on the page moves when it comes or goes; "later"
 * keeps it away for a month.
 */
export function InstallBanner() {
  const { t } = useTranslation("pwa");
  const way = useInstallWay();
  const online = useOnline();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [snoozed, setSnoozed] = useState(installOfferSnoozed);
  const [settled, setSettled] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(true), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, []);

  const offered = way === "prompt" || hasGuide(way);
  if (!offered || snoozed || !settled || !online || !CALM_PAGES.has(pathname.replace(/\/+$/, "")))
    return null;

  const dismiss = () => {
    snoozeInstallOffer();
    setSnoozed(true);
  };
  const install = async () => {
    if (way !== "prompt") {
      openInstallGuide();
      return;
    }
    setBusy(true);
    const outcome = await promptInstall();
    setBusy(false);
    // "No" to the browser's dialog is an answer too: do not ask again right away.
    if (outcome === "dismissed") dismiss();
  };

  return (
    <aside
      aria-label={t("banner.label")}
      className="pointer-events-none absolute inset-x-0 bottom-0 z-40 flex justify-center px-4 pb-[calc(max(env(safe-area-inset-bottom),0.75rem)+5rem)] md:pb-6"
    >
      <div className="sticker pointer-events-auto relative flex w-full max-w-md animate-slide-up items-center gap-3 rounded-3xl bg-surface p-3">
        <img
          src="/icon.svg"
          alt=""
          className="size-11 shrink-0 rounded-[22%] border-2 border-ink"
        />
        <div className="min-w-0 flex-1 [word-break:auto-phrase]">
          <p className="font-black leading-snug">{t("banner.title")}</p>
          <p className="text-[13px] font-normal leading-snug text-ink-soft">
            {way === "ios-in-app" ? t("banner.inAppBody") : t("banner.body")}
          </p>
        </div>
        <Button size="sm" className="shrink-0" loading={busy} onClick={install}>
          {way === "prompt" ? t("banner.install") : t("banner.howTo")}
        </Button>
        {/* On the corner, so it takes no room from the text; the ring around it widens what a finger can hit. */}
        <button
          type="button"
          aria-label={t("banner.dismiss")}
          onClick={dismiss}
          className="sticker-sm absolute -top-3 -right-2 grid size-8 place-items-center rounded-full bg-surface text-ink-soft before:absolute before:-inset-2 before:content-[''] hover:text-ink"
        >
          <X className="size-4" />
        </button>
      </div>
    </aside>
  );
}
