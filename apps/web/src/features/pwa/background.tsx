import { Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { onDemand } from "#/components/on-demand";
import { useAppBadge } from "./app-badge";
import { useInstall } from "./install";
import { InstallBanner } from "./install-banner";
import { OfflineIndicator } from "./offline-indicator";
import { useProtocolWatch } from "./protocol";
import { startServiceWorker } from "./service-worker";
import "./standalone.css";
import { useThemeColor } from "./theme-color";
import { useUpdateToast } from "./update-toast";

const InstallGuide = onDemand(
  () => import("./install-guide").then((module) => module.InstallGuide),
  "toast",
);

/**
 * Runs for as long as someone is signed in: everything that makes the web
 * app behave like an installed one. The service worker (offline page, caches,
 * the "new version" toast), the unread count on the app's icon, the colour of
 * the bars around the page, the offline pill, and the offer to install.
 */
export default function PwaBackground() {
  const { i18n } = useTranslation("pwa");
  // The guide loads the first time it opens, and stays mounted so it can animate out.
  const guideOpen = useInstall((state) => state.guideOpen);
  const [guideOpened, setGuideOpened] = useState(false);
  if (guideOpen && !guideOpened) setGuideOpened(true);

  useEffect(startServiceWorker, []);
  useAppBadge();
  useThemeColor();
  useUpdateToast();
  useProtocolWatch();

  // Said once, when it happens: not again when the language changes.
  useEffect(() => {
    const t = i18n.getFixedT(null, "pwa");
    return useInstall.subscribe((state, previous) => {
      if (state.justInstalled && !previous.justInstalled)
        toast.success(t("install.installedToast"));
    });
  }, [i18n]);

  return (
    <>
      <OfflineIndicator />
      <InstallBanner />
      {guideOpened && (
        <Suspense fallback={null}>
          <InstallGuide />
        </Suspense>
      )}
    </>
  );
}
