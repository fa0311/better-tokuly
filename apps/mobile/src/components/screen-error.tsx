import { type ErrorBoundaryProps, useNavigation } from "expo-router";
import { useEffect, useState } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { View } from "react-native";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { Button } from "./button";
import { EmptyState } from "./page";

/*
 * What is shown when rendering threw. React Native already logs every error
 * a boundary catches, once, with its component stack (`ExceptionsManager`:
 * the console in development, the device log in a release build), so these
 * only show it.
 */

/**
 * A screen that threw, in its place: every route of the app has it (the root
 * `Stack`'s `unstable_screenErrorBoundary`, inherited by every navigator under
 * it), so the tab bar, the header and the other screens stay. "もう一度"
 * renders the screen again; "もどる" leaves it.
 */
export function ScreenError({ retry }: ErrorBoundaryProps) {
  const navigation = useNavigation();
  // When a pushed screen first renders, its navigator's state does not have it yet.
  const [canGoBack, setCanGoBack] = useState(false);
  useEffect(() => {
    const update = () => setCanGoBack(navigation.canGoBack());
    update();
    return navigation.addListener("state", update);
  }, [navigation]);
  return <Failed retry={retry} back={canGoBack ? () => navigation.goBack() : undefined} />;
}

/**
 * What failed above every screen (the root layout, a group's layout): the
 * root route's `ErrorBoundary`. It is outside the root's providers, so it
 * brings the strings and the theme itself.
 */
export function RootError({ retry }: ErrorBoundaryProps) {
  const theme = useTheme();
  return (
    <I18nextProvider i18n={i18n}>
      <View style={[{ flex: 1 }, theme.variables]} className="bg-bg">
        <Failed retry={retry} />
      </View>
    </I18nextProvider>
  );
}

function Failed({ retry, back }: { retry(): Promise<void>; back?(): void }) {
  const { t } = useTranslation("mobile");
  return (
    <View testID="screen-error" accessibilityRole="alert" className="flex-1 justify-center bg-bg">
      <EmptyState
        emoji="😵‍💫"
        title={t("failed.title")}
        body={t("failed.body")}
        action={
          <View className="items-center gap-3">
            <Button testID="screen-error-retry" onPress={() => void retry()}>
              {t("actions.retry", { ns: "common" })}
            </Button>
            {back && (
              <Button testID="screen-error-back" variant="secondary" onPress={back}>
                {t("actions.back", { ns: "common" })}
              </Button>
            )}
          </View>
        }
      />
    </View>
  );
}
