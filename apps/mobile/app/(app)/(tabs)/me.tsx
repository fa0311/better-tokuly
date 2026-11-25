import { Link } from "expo-router";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/button";
import { ProfileScreen } from "#/features/profile/profile-screen";
import { useMe } from "#/features/session/session";

/** The own profile, with the way to settings and accounts. */
export default function Me() {
  const { t } = useTranslation("mobile");
  const me = useMe();
  return (
    <ProfileScreen
      handle={me?.user?.handle ?? ""}
      footer={
        <Link href="/settings" asChild>
          <Button testID="me-settings" variant="secondary" block>
            {t("settings.title")}
          </Button>
        </Link>
      }
    />
  );
}
