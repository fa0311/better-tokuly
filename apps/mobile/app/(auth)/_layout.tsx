import { Redirect, Stack } from "expo-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { routeFor, usePendingLink } from "#/features/session/routing";
import { useSessionStore } from "#/features/session/store";
import { useTheme } from "#/lib/theme";

export const unstable_settings = { initialRouteName: "welcome" };

/** Signed out: welcome, sign-in, sign-up. Someone signed in goes on to the app (or the link they opened). */
export default function AuthLayout() {
  const status = useSessionStore((s) => s.state.status);
  const link = usePendingLink((s) => s.link);
  const to = routeFor(status, "auth", link);
  useEffect(() => {
    if (to) usePendingLink.getState().clear();
  }, [to]);
  if (to) return <Redirect href={to} />;
  return <AuthStack />;
}

function AuthStack() {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  return (
    <Stack
      screenOptions={{
        headerTintColor: colors.ink,
        headerStyle: { backgroundColor: colors.bg },
        headerShadowVisible: false,
        headerTitle: "",
        contentStyle: { backgroundColor: colors.bg },
      }}
    >
      <Stack.Screen name="welcome" options={{ headerShown: false }} />
      <Stack.Screen name="sign-in" options={{ headerBackTitle: t("welcome.signIn") }} />
      <Stack.Screen name="sign-up" />
    </Stack>
  );
}
