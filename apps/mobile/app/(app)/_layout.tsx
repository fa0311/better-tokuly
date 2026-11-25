import { Stack } from "expo-router";
import { useTranslation } from "react-i18next";
import { useFriendsSync } from "#/features/friends/data";
import { MapBackground } from "#/features/map/background";
import { useTheme } from "#/lib/theme";

/** Signed in: the tabs, the screens pushed over them, and what runs on every one of them. */
export const unstable_settings = { initialRouteName: "(tabs)" };

export default function AppLayout() {
  const { t } = useTranslation("mobile");
  const { t: ta } = useTranslation("accounts");
  const { colors } = useTheme();
  useFriendsSync();
  return (
    <>
      <MapBackground />
      <Stack
        screenOptions={{
          headerTintColor: colors.ink,
          headerStyle: { backgroundColor: colors.bg },
          headerShadowVisible: false,
          headerTitleStyle: { fontWeight: "900" },
          contentStyle: { backgroundColor: colors.bg },
        }}
      >
        <Stack.Screen name="(tabs)" options={{ headerShown: false }} />
        <Stack.Screen
          name="add-friend"
          options={{ title: t("friends.addTitle"), presentation: "modal" }}
        />
        <Stack.Screen name="settings" options={{ title: t("settings.title") }} />
        <Stack.Screen name="accounts" options={{ title: ta("title") }} />
        <Stack.Screen
          name="add-account"
          options={{ title: ta("addTitle"), presentation: "modal" }}
        />
        <Stack.Screen
          name="create-account"
          options={{ title: ta("create"), presentation: "modal" }}
        />
        <Stack.Screen name="u/[handle]" options={{ title: "" }} />
        <Stack.Screen name="invite/[code]" options={{ title: "" }} />
      </Stack>
    </>
  );
}
