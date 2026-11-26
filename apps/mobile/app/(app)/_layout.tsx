import { Redirect, Stack, usePathname } from "expo-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ChatBackground } from "#/features/chat/background";
import { useFriendsSync } from "#/features/friends/data";
import { MapBackground } from "#/features/map/background";
import { PostsBackground } from "#/features/media/background";
import { NotificationsBackground } from "#/features/notifications/background";
import { routeFor, usePendingLink } from "#/features/session/routing";
import { useSessionStore } from "#/features/session/session";
import { StoriesBackground } from "#/features/stories/background";
import { useTheme } from "#/lib/theme";

/** Signed in: the tabs, the screens pushed over them, and what runs on every one of them. */
export const unstable_settings = { initialRouteName: "(tabs)" };

/** Someone signed out who opened a link in here signs in first; the link waits for them. */
export default function AppLayout() {
  const status = useSessionStore((s) => s.state.status);
  const path = usePathname();
  const to = routeFor(status, "app");
  useEffect(() => {
    if (to) usePendingLink.getState().remember(path);
  }, [to, path]);
  if (to) return <Redirect href={to} />;
  return <SignedIn />;
}

function SignedIn() {
  const { t } = useTranslation("mobile");
  const { t: ta } = useTranslation("accounts");
  const { t: ts } = useTranslation("stories");
  const { t: tsh } = useTranslation("shorts");
  const { colors } = useTheme();
  useFriendsSync();
  return (
    <>
      <MapBackground />
      <ChatBackground />
      <NotificationsBackground />
      <StoriesBackground />
      <PostsBackground />
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
        <Stack.Screen name="licenses" options={{ title: t("licenses.title") }} />
        <Stack.Screen
          name="add-account"
          options={{ title: ta("addTitle"), presentation: "modal" }}
        />
        <Stack.Screen
          name="create-account"
          options={{ title: ta("create"), presentation: "modal" }}
        />
        <Stack.Screen name="chat/[id]" options={{ headerShown: false }} />
        <Stack.Screen
          name="new-chat"
          options={{ title: t("chat.newChat"), presentation: "modal" }}
        />
        <Stack.Screen name="u/[handle]" options={{ title: "" }} />
        <Stack.Screen
          name="stories/[userId]"
          options={{ headerShown: false, presentation: "fullScreenModal", animation: "fade" }}
        />
        <Stack.Screen
          name="stories/new"
          options={{ title: ts("create.title"), presentation: "modal" }}
        />
        <Stack.Screen
          name="shorts/watch"
          options={{
            title: "",
            headerTransparent: true,
            headerTintColor: "#ffffff",
            contentStyle: { backgroundColor: "#000000" },
          }}
        />
        <Stack.Screen name="shorts/tag/[tag]" options={{ title: "" }} />
        <Stack.Screen name="shorts/sound/[id]" options={{ title: tsh("sound.pageTitle") }} />
        <Stack.Screen
          name="shorts/new"
          options={{ title: tsh("create.title"), presentation: "modal" }}
        />
        <Stack.Screen name="shorts/mine" options={{ title: tsh("studio.title") }} />
        <Stack.Screen name="invite/[code]" options={{ title: "" }} />
      </Stack>
    </>
  );
}
