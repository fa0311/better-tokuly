import { Redirect, Stack, usePathname } from "expo-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { routeFor, usePendingLink } from "#/features/session/routing";
import { useSessionStore } from "#/features/session/store";
import { BirthdateGate } from "#/features/settings/birthdate-gate";
import { useTheme } from "#/lib/theme";
import { BackgroundFeatures } from "#/seams/background";
import { BadgeSourceRunner } from "#/seams/badges";
import { SCREENS } from "#/seams/screens";

/**
 * Signed in: the tabs, the screens pushed over them, and what runs on every one of them.
 * All three come from the features (`#/seams`): nothing is listed here. An
 * account without a birthdate is asked for it first (`BirthdateGate`).
 */
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
  // The titles are read below in the language of this render.
  useTranslation();
  const { colors } = useTheme();
  return (
    <>
      <BackgroundFeatures />
      <BadgeSourceRunner />
      <BirthdateGate>
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
          {SCREENS.map((screen) => (
            <Stack.Screen key={screen.name} name={screen.name} options={screen.options()} />
          ))}
        </Stack>
      </BirthdateGate>
    </>
  );
}
