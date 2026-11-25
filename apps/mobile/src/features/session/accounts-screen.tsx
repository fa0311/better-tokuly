import { canAddAccount } from "@tokuly/core/session";
import { Link } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { Page, Section, Spinner } from "#/components/page";
import { AccountList } from "./account-list";
import { useDeviceAccounts } from "./device-accounts";

/**
 * The accounts on this device (a main one, a 裏垢, a creator account) and
 * switching between them. Each keeps its own sign-in; none of them can see
 * the others.
 */
export function AccountsScreen() {
  const { t } = useTranslation("accounts");
  const { data, isPending } = useDeviceAccounts();
  return (
    <Page testID="accounts-screen">
      <Text className="text-sm text-ink-soft">{t("lead")}</Text>
      <Section title={t("listTitle")}>
        {isPending ? <Spinner /> : <AccountList accounts={data?.accounts ?? []} />}
      </Section>
      {canAddAccount(data) ? (
        <View className="flex-row flex-wrap gap-2">
          <Link href="/add-account" asChild>
            <Button testID="accounts-add">{t("add")}</Button>
          </Link>
          <Link href="/create-account" asChild>
            <Button variant="secondary">{t("create")}</Button>
          </Link>
        </View>
      ) : (
        <Text className="px-1 text-sm font-bold text-ink-soft">
          {t("full", { count: data?.limit ?? 0 })}
        </Text>
      )}
      <Text className="px-1 text-sm text-ink-soft">{t("privacyNote")}</Text>
    </Page>
  );
}
