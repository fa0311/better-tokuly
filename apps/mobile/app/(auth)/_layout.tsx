import { Stack } from "expo-router";
import { useTranslation } from "react-i18next";
import { useTheme } from "#/lib/theme";

export default function AuthLayout() {
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
