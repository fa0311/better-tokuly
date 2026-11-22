import { Link } from "@tanstack/react-router";
import { ChevronRight, MonitorSmartphone } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMe } from "#/stores/session";
import { useFreshMe } from "../data";
import { SettingsPage } from "../shell/settings-page";
import { PasskeysCard } from "./passkeys-card";
import { RecentSecurityEvents } from "./security-log";
import { SessionsCard } from "./sessions-card";
import { TotpCard } from "./totp-card";

/** How the account is protected: passkeys, two-step verification, devices, the log. */
export function SecuritySection() {
  const { t } = useTranslation("settings");
  const me = useMe();
  useFreshMe();
  if (!me) return null;
  return (
    <SettingsPage title={t("sections.security.title")}>
      <PasskeysCard />
      <TotpCard me={me} />
      <SessionsCard />
      <Link
        to="/settings/security/device"
        className="pressable sticker mt-7 flex items-center gap-3 rounded-card bg-surface p-4"
      >
        <span className="grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink bg-lime text-[#17122b]">
          <MonitorSmartphone className="size-5" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block font-black">{t("security.device.linkTitle")}</span>
          <span className="block text-sm text-ink-soft">{t("security.device.linkHint")}</span>
        </span>
        <ChevronRight className="size-5 shrink-0 text-ink-faint" />
      </Link>
      <RecentSecurityEvents />
    </SettingsPage>
  );
}
