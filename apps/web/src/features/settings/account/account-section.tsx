import { useTranslation } from "react-i18next";
import { useMe } from "#/stores/session";
import { useFreshMe } from "../data";
import { SettingsPage } from "../shell/settings-page";
import { EmailCard } from "./email-card";
import { PasswordCard } from "./password-card";
import { RegionCard } from "./region-card";

/** How you sign in and how Tokuly talks to you: email, password, language, time zone. */
export function AccountSection() {
  const { t } = useTranslation("settings");
  const me = useMe();
  useFreshMe();
  if (!me) return null;
  return (
    <SettingsPage title={t("sections.account.title")}>
      <EmailCard me={me} />
      <PasswordCard me={me} />
      <RegionCard me={me} />
    </SettingsPage>
  );
}
