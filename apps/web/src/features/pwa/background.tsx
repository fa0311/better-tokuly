import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useAppBadge } from "./app-badge";
import { useInstall } from "./install";
import { InstallBanner } from "./install-banner";
import { InstallGuide } from "./install-guide";
import { OfflineIndicator } from "./offline-indicator";
import { startServiceWorker } from "./service-worker";
import "./standalone.css";
import { useThemeColor } from "./theme-color";
import { useUpdateToast } from "./update-toast";

/**
 * Runs for as long as someone is signed in: everything that makes the web
 * app behave like an installed one. The service worker (offline page, caches,
 * the "new version" toast), the unread count on the app's icon, the colour of
 * the bars around the page, the offline pill, and the offer to install.
 */
export default function PwaBackground() {
  const { t } = useTranslation("pwa");
  const justInstalled = useInstall((state) => state.justInstalled);

  useEffect(startServiceWorker, []);
  useAppBadge();
  useThemeColor();
  useUpdateToast();

  useEffect(() => {
    if (justInstalled) toast.success(t("install.installedToast"));
  }, [justInstalled, t]);

  return (
    <>
      <OfflineIndicator />
      <InstallBanner />
      <InstallGuide />
    </>
  );
}
