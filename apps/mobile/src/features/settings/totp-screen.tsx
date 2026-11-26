import { useMutation, useQuery } from "@tanstack/react-query";
import { errorReason, identity } from "@tokuly/api";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Share, Text, View } from "react-native";
import { Button } from "#/components/button";
import { ErrorNote, Page, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { type Account, currentAccount } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { adoptTotp } from "./data";

/** Keeps a code field to what a code can be: digits, optionally spaced. */
const digitsOnly = (value: string) => value.replace(/[^\d\s]/g, "").slice(0, 7);

/**
 * Turning two-step verification on. On a phone there is nothing to scan: the
 * key opens in the authenticator app on the same phone (`otpauth://`), or is
 * typed in by hand. The recovery codes show once, right after.
 */
export function TotpSetupScreen() {
  const { t } = useTranslation(["settings", "mobile", "common"]);
  const me = useMe();
  // The whole setup belongs to the account that began it.
  const [account] = useState<Account | null>(currentAccount);
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [noApp, setNoApp] = useState(false);
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);

  // Beginning creates the secret (valid for ten minutes); it is never silently replaced.
  const setup = useQuery({
    queryKey: ["identity", "totp-setup"],
    queryFn: () => {
      if (!account) throw new Error("signed out");
      return account.api.service(identity.AccountService).beginTotpSetup({});
    },
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
    retry: false,
    refetchOnReconnect: false,
  });

  const enable = useMutation({
    mutationFn: async () => {
      if (!account) throw new Error("signed out");
      const service = account.api.service(identity.AccountService);
      const response = await service.enableTotp({ code, currentPassword: password });
      await adoptTotp(account, true);
      return response.recoveryCodes;
    },
    onSuccess: setRecoveryCodes,
    onError: (failure) => {
      const reason = errorReason(failure);
      setError(
        reason === "INVALID_CREDENTIALS"
          ? t("settings:account.password.wrongCurrent")
          : errorMessage(failure, { namespace: "settings" }),
      );
      // The key ran out (ten minutes): no code for it will ever do, so the setup starts over with a new one.
      if (reason === "TOTP_SETUP_EXPIRED") {
        setCode("");
        void setup.refetch();
      }
    },
  });

  if (recoveryCodes) {
    const text = recoveryCodes.join("\n");
    return (
      <Page testID="totp-recovery">
        <Text className="text-lg font-black text-ink">
          {t("settings:security.totp.recoveryTitle")}
        </Text>
        <Text className="text-sm text-ink-soft">{t("settings:security.totp.recoveryLead")}</Text>
        <View testID="recovery-codes" className="flex-row flex-wrap rounded-card bg-surface p-4">
          {recoveryCodes.map((recovery) => (
            <Text
              key={recovery}
              selectable
              className="w-1/2 py-1 text-center font-mono text-base font-bold tracking-wider text-ink"
            >
              {recovery}
            </Text>
          ))}
        </View>
        <Button
          variant="secondary"
          onPress={() =>
            void Share.share({
              message: `${t("settings:security.totp.recoveryFileHeader")}\n\n${text}\n`,
            })
          }
        >
          {t("mobile:totp.shareCodes")}
        </Button>
        <Button testID="totp-recovery-done" onPress={() => router.back()}>
          {t("settings:security.totp.recoveryDone")}
        </Button>
      </Page>
    );
  }

  return (
    <Page testID="totp-setup">
      <Text className="text-sm text-ink-soft">{t("settings:security.totp.setupLead")}</Text>
      {setup.isError ? (
        <View className="gap-3">
          <ErrorNote>{errorMessage(setup.error, { namespace: "settings" })}</ErrorNote>
          <Button variant="secondary" size="sm" onPress={() => void setup.refetch()}>
            {t("common:actions.retry")}
          </Button>
        </View>
      ) : !setup.data ? (
        <Spinner />
      ) : (
        <>
          <View className="gap-3 rounded-card bg-surface p-4">
            <Text className="font-black text-ink">{t("mobile:totp.step1")}</Text>
            <Button
              testID="totp-open-app"
              variant="lime"
              onPress={() =>
                void Linking.openURL(setup.data.otpauthUrl).catch(() => setNoApp(true))
              }
            >
              {t("mobile:totp.openApp")}
            </Button>
            <Text className="text-xs text-ink-soft">
              {noApp ? t("mobile:totp.noApp") : t("mobile:totp.manual")}
            </Text>
            <Text
              testID="totp-secret"
              selectable
              className="rounded-control bg-surface-2 px-4 py-3 font-mono text-sm font-bold tracking-wider text-ink"
            >
              {setup.data.secret}
            </Text>
          </View>
          <View className="gap-3 rounded-card bg-surface p-4">
            <TextField
              testID="totp-code"
              label={t("settings:security.totp.step2")}
              value={code}
              onChangeText={(value) => setCode(digitsOnly(value))}
              keyboardType="number-pad"
              autoComplete="one-time-code"
              placeholder="123456"
            />
            <Text className="px-1 text-xs text-ink-soft">
              {t("settings:security.totp.codeHint")}
            </Text>
            {/* Turning this on takes the password, so a session alone cannot lock the account. */}
            {me?.hasPassword && (
              <TextField
                testID="totp-password"
                label={t("settings:account.password.current")}
                value={password}
                onChangeText={setPassword}
                secureTextEntry
                autoComplete="current-password"
              />
            )}
            {error && <ErrorNote>{error}</ErrorNote>}
            <Button
              testID="totp-confirm"
              loading={enable.isPending}
              disabled={
                code.replace(/\s/g, "").length !== 6 || (Boolean(me?.hasPassword) && !password)
              }
              onPress={() => {
                setError(null);
                enable.mutate();
              }}
            >
              {t("settings:security.totp.confirm")}
            </Button>
          </View>
        </>
      )}
    </Page>
  );
}
