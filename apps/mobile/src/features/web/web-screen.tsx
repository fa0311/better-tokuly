import { useNavigation } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, BackHandler, Linking, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { WebView, type WebViewNavigation } from "react-native-webview";
import { Button } from "#/components/button";
import { AccountChanged, currentAccount } from "#/features/media/account";
import { useSessionStore } from "#/features/session/session";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { requestWebSession, staysInApp, VERIFIER_HEADER, type WebSession } from "./handoff";

/**
 * The web view without extra props. Its default (`P = undefined`) makes the
 * props `WebViewProps & undefined`, which is `never` to TypeScript 7.
 */
const TokulyWebView = WebView<object>;

type Opening =
  | { status: "loading" }
  | { status: "ready"; session: WebSession }
  | { status: "failed"; message: string };

/** The session of the account in use, or `null`. */
function useActiveSessionId(): string | null {
  return useSessionStore((s) => (s.state.status === "signedIn" ? s.state.sessionId : null));
}

/**
 * A screen of the web app shown in the app, signed in as the account in use.
 *
 * Opening it asks for a one-time hand-off code as that account
 * (`requestWebSession`) and loads it: the web view never sees the app's
 * tokens. Nothing of one account reaches another:
 * - a code that comes back after another account took over is dropped, and
 *   the web view only shows while its account is still the active one;
 * - the web view is incognito and keyed by its account's session: each one
 *   starts with no cookies (iOS: a store of its own that is never written
 *   to disk; Android: the cookie store is emptied whenever one opens) and
 *   the app's own API calls never carry cookies (`lib/api.ts`), so the web
 *   session's cookie stays in its web view;
 * - a switch or a sign-out starts the signed-in screens over, which closes
 *   it, and the server ends the web session with the app's session.
 *
 * Links to anywhere but this server open in the system browser.
 */
export function WebScreen({ path }: { path: string }) {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const navigation = useNavigation();
  const activeSessionId = useActiveSessionId();
  const [opening, setOpening] = useState<Opening>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const web = useRef<WebView<object>>(null);
  const canGoBack = useRef(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `attempt` is the retry button: a new code each press.
  useEffect(() => {
    // The account is the one in use now, before anything awaits.
    const account = currentAccount();
    if (!account) return;
    let current = true;
    setOpening({ status: "loading" });
    requestWebSession(account, path).then(
      (session) => {
        if (current) setOpening({ status: "ready", session });
      },
      (error: unknown) => {
        if (current && !(error instanceof AccountChanged))
          setOpening({ status: "failed", message: errorMessage(error) });
      },
    );
    return () => {
      current = false;
    };
  }, [path, attempt]);

  // Android's back button goes back in the web view first.
  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (!canGoBack.current) return false;
      web.current?.goBack();
      return true;
    });
    return () => subscription.remove();
  }, []);

  const onNavigation = (state: WebViewNavigation) => {
    canGoBack.current = state.canGoBack;
    if (state.title && !state.loading) navigation.setOptions({ title: state.title });
  };

  if (opening.status === "failed")
    return (
      <View testID="web-failed" className="flex-1 items-center justify-center gap-3 bg-bg px-6">
        <Text className="text-base font-black text-ink">{t("web.failed")}</Text>
        <Text className="text-center text-sm text-ink-soft">{opening.message}</Text>
        <Button variant="secondary" onPress={() => setAttempt((n) => n + 1)}>
          {t("web.retry")}
        </Button>
      </View>
    );
  // Only ever the active account's web view.
  if (opening.status === "loading" || opening.session.sessionId !== activeSessionId)
    return (
      <View testID="web-loading" className="flex-1 items-center justify-center gap-3 bg-bg">
        <ActivityIndicator color={colors.brand} />
        <Text className="text-sm text-ink-soft">{t("web.loading")}</Text>
      </View>
    );
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg">
      <TokulyWebView
        key={opening.session.sessionId}
        ref={web}
        testID="web-view"
        // The verifier goes with the first load only (react-native-webview
        // sends `headers` on the initial request, not on the redirect or later pages).
        source={{
          uri: opening.session.url,
          headers: { [VERIFIER_HEADER]: opening.session.verifier },
        }}
        incognito
        sharedCookiesEnabled={false}
        thirdPartyCookiesEnabled={false}
        // `target="_blank"` stays in this web view, where the check below sees it.
        setSupportMultipleWindows={false}
        onShouldStartLoadWithRequest={(request) => {
          if (staysInApp(request.url)) return true;
          void Linking.openURL(request.url).catch(() => {});
          return false;
        }}
        onNavigationStateChange={onNavigation}
        startInLoadingState
        renderLoading={() => (
          <View className="absolute inset-0 items-center justify-center bg-bg">
            <ActivityIndicator color={colors.brand} />
          </View>
        )}
        style={{ flex: 1, backgroundColor: colors.bg }}
      />
    </SafeAreaView>
  );
}
