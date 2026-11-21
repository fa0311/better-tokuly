import { useTranslation } from "react-i18next";
import { useMe } from "#/stores/session";
import { SettingsPage } from "../shell/settings-page";
import { ProfileForm } from "./profile-form";
import { ProfileImages } from "./profile-images";
import { StatusCard } from "./status-card";

/** Everything other people see about you: pictures, status, name, handle, bio. */
export function ProfileSection() {
  const { t } = useTranslation("settings");
  const me = useMe();
  if (!me?.user) return null;
  return (
    <SettingsPage title={t("sections.profile.title")}>
      <ProfileImages me={me} />
      <StatusCard me={me} />
      <ProfileForm me={me} />
    </SettingsPage>
  );
}
