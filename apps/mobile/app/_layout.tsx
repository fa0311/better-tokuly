import "../global.css";
import { SplashScreen, Stack } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect } from "react";
import { I18nextProvider } from "react-i18next";
import { View } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { RootError, ScreenError } from "#/components/screen-error";
import { AccountScope } from "#/features/session/account-scope";
import { startSession } from "#/features/session/session";
import { useSessionStore } from "#/features/session/store";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { touchLogProps } from "#/lib/touch-log";

void SplashScreen.preventAutoHideAsync();
void startSession();

/**
 * The root: theme, strings, and the stack of whoever is signed in. While the
 * session is being restored the splash screen stays up. Where each start
 * lands is `routeFor` (`features/session/routing.ts`): `/` and the group
 * layouts redirect by it, so a signed-in account sees `(app)`, everyone else
 * `(auth)`, whatever link opened the app.
 *
 * A screen that throws while rendering shows `ScreenError` in its place: every
 * navigator under this stack inherits it, so the tabs and the other screens stay.
 */
export default function RootLayout() {
  const theme = useTheme();
  const status = useSessionStore((s) => s.state.status);
  const generation = useSessionStore((s) => s.generation);
  useEffect(() => {
    if (status !== "unknown") void SplashScreen.hideAsync();
  }, [status]);
  if (status === "unknown") return null;
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <View style={[{ flex: 1 }, theme.variables]} {...touchLogProps}>
        <SafeAreaProvider>
          <I18nextProvider i18n={i18n}>
            <AccountScope key={generation}>
              <StatusBar style={theme.dark ? "light" : "dark"} />
              <Stack
                key={generation}
                unstable_screenErrorBoundary={ScreenError}
                screenOptions={{
                  headerShown: false,
                  contentStyle: { backgroundColor: theme.colors.bg },
                }}
              >
                <Stack.Screen name="index" />
                <Stack.Screen name="(app)" />
                <Stack.Screen name="(auth)" />
                <Stack.Screen name="+not-found" />
              </Stack>
            </AccountScope>
          </I18nextProvider>
        </SafeAreaProvider>
      </View>
    </GestureHandlerRootView>
  );
}

/** What throws above the screens (this layout, a group's) shows `RootError` instead of a blank app. */
export const ErrorBoundary = RootError;
