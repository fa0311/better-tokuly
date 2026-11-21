import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import { ProfileSection } from "../profile/profile-section";
import { IdentityCard, SettingsNav, SignOutButton } from "./settings-nav";
import { useIsWide } from "./use-is-wide";

/**
 * `/settings`. On phones this is the list of sections; on wide screens the
 * list is always on the left, so the first section opens beside it.
 */
export function SettingsHome() {
  const { t } = useTranslation("settings");
  const wide = useIsWide();
  if (wide) return <ProfileSection />;
  return (
    <Page title={t("title")} back="/me">
      <IdentityCard />
      <div className="sticker mt-4 rounded-card bg-surface p-2">
        <SettingsNav />
      </div>
      <SignOutButton className="mt-6" />
    </Page>
  );
}
