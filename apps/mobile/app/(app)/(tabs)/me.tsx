import { Link } from "expo-router";
import { useTranslation } from "react-i18next";
import { View } from "react-native";
import { Button } from "#/components/button";
import { ProfileScreen } from "#/features/profile/profile-screen";
import { useMe } from "#/features/session/store";
import { HUB_ENTRIES } from "#/seams/hub";

/** The own profile, with the hub under it: the way into every area that has no tab. */
export default function Me() {
  // The labels are read below in the language of this render.
  useTranslation();
  const me = useMe();
  return (
    <ProfileScreen
      handle={me?.user?.handle ?? ""}
      footer={
        <View className="gap-2">
          {HUB_ENTRIES.map((entry) => (
            <Link key={entry.id} href={entry.href} asChild>
              <Button testID={`me-${entry.id}`} variant="secondary" block>
                {entry.label()}
              </Button>
            </Link>
          ))}
        </View>
      }
    />
  );
}
