import { Link } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { LogoMark } from "#/components/logo";
import { Section } from "#/components/page";
import { AccountList } from "./account-list";
import { useDeviceAccounts } from "./device-accounts";

/**
 * The first screen without an active account: what Tokuly is, a way in, and
 * the accounts still parked on this device (continue as one of them).
 */
export function WelcomeScreen() {
  const { t } = useTranslation("auth");
  const { t: tm } = useTranslation("mobile");
  const { t: tc } = useTranslation("common");
  const { data } = useDeviceAccounts();
  const parked = data?.accounts ?? [];
  return (
    <SafeAreaView className="flex-1 bg-bg" testID="welcome-screen">
      <View className="flex-1 justify-center gap-3 px-6">
        <LogoMark size={88} />
        <Text className="text-5xl font-black tracking-tight text-ink">{tc("appName")}</Text>
        <Text className="text-2xl font-black text-brand">{t("landing.headline")}</Text>
        <Text className="text-base text-ink-soft">{tc("tagline")}</Text>
      </View>
      <View className="gap-4 px-6 pb-8">
        {parked.length > 0 && (
          <Section title={tm("accounts.continueTitle")}>
            <AccountList accounts={parked} />
          </Section>
        )}
        <Link href="/sign-up" asChild>
          <Button testID="welcome-sign-up" size="lg" block>
            {tm("welcome.signUp")}
          </Button>
        </Link>
        <Link href="/sign-in" asChild>
          <Button testID="welcome-sign-in" size="lg" variant="secondary" block>
            {tm("welcome.signIn")}
          </Button>
        </Link>
      </View>
    </SafeAreaView>
  );
}
