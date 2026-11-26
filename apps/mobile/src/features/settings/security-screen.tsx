import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { common, identity, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { router } from "expo-router";
import {
  Fingerprint,
  Globe,
  Laptop,
  MonitorSmartphone,
  ScrollText,
  Smartphone,
} from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Button } from "#/components/button";
import { Page, Row, Section, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { currentAccount, isActive } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";

export const securityKeys = {
  sessions: ["identity", "sessions"] as const,
};

/**
 * How the account is protected: two-step verification and the devices
 * signed in to it, as on the web. Passkeys, approving another device's
 * sign-in and the log open on the web.
 */
export function SecurityScreen() {
  const { t } = useTranslation("settings");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const web = (path: string) => () => router.push(webScreenHref(path) as never);
  return (
    <Page testID="security-screen">
      <TotpSection />
      <SessionsSection />
      <Section>
        <Row
          testID="security-passkeys"
          icon={<Fingerprint size={20} color={colors.ink} />}
          label={t("security.passkeys.title")}
          hint={tm("security.onWeb")}
          onPress={web("/settings/security")}
        />
        <Row
          testID="security-device"
          icon={<MonitorSmartphone size={20} color={colors.ink} />}
          label={t("security.device.linkTitle")}
          hint={t("security.device.linkHint")}
          onPress={web("/settings/security/device")}
        />
        <Row
          testID="security-log"
          icon={<ScrollText size={20} color={colors.ink} />}
          label={t("security.log.title")}
          hint={tm("security.onWeb")}
          onPress={web("/settings/security/log")}
        />
      </Section>
    </Page>
  );
}

/** Two-step verification: on (set up on its own screen) or off (with a code). */
function TotpSection() {
  const { t } = useTranslation(["settings", "common"]);
  const me = useMe();
  const [disabling, setDisabling] = useState(false);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const disable = useMutation({
    mutationFn: async () => {
      const account = currentAccount();
      if (!account) return;
      await account.api.service(identity.AccountService).disableTotp({ code: code.trim() });
      const fresh = await account.api.service(identity.AccountService).getMe({});
      if (fresh.me && isActive(account)) account.api.session.updateMe(fresh.me);
    },
    onSuccess: () => {
      setDisabling(false);
      setCode("");
      Alert.alert(t("settings:security.totp.disabled"));
    },
    onError: (failure) => setError(errorMessage(failure, { namespace: "settings" })),
  });
  if (!me) return null;
  const on = me.totpEnabled;
  return (
    <Section title={t("settings:security.totp.title")} lead={t("settings:security.totp.lead")}>
      <View className="gap-3 p-4">
        <View className="flex-row items-center gap-3">
          <View className={`rounded-full px-3 py-1 ${on ? "bg-mint" : "bg-surface-3"}`}>
            <Text testID="totp-state" className="text-xs font-black text-ink">
              {on ? t("settings:security.totp.on") : t("settings:security.totp.off")}
            </Text>
          </View>
          <Text className="flex-1 text-sm text-ink-soft">
            {on ? t("settings:security.totp.onHint") : t("settings:security.totp.offHint")}
          </Text>
        </View>
        {!on && (
          <Button
            testID="totp-enable"
            size="sm"
            variant="lime"
            onPress={() => router.push("/settings/totp")}
          >
            {t("settings:security.totp.enable")}
          </Button>
        )}
        {on && !disabling && (
          <Button
            testID="totp-disable"
            size="sm"
            variant="secondary"
            onPress={() => setDisabling(true)}
          >
            {t("settings:security.totp.disable")}
          </Button>
        )}
        {on && disabling && (
          <View className="gap-3">
            <Text className="text-sm text-ink-soft">{t("settings:security.totp.disableLead")}</Text>
            <TextField
              testID="totp-disable-code"
              label={t("settings:security.totp.disableCode")}
              value={code}
              onChangeText={setCode}
              autoCapitalize="none"
              autoComplete="one-time-code"
              autoCorrect={false}
              autoFocus
            />
            <Text className="px-1 text-xs text-ink-soft">
              {error ?? t("settings:security.totp.disableCodeHint")}
            </Text>
            <View className="flex-row justify-end gap-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={disable.isPending}
                onPress={() => {
                  setDisabling(false);
                  setError(null);
                }}
              >
                {t("common:actions.cancel")}
              </Button>
              <Button
                testID="totp-disable-confirm"
                size="sm"
                variant="danger"
                loading={disable.isPending}
                disabled={!code.trim()}
                onPress={() => {
                  setError(null);
                  disable.mutate();
                }}
              >
                {t("settings:security.totp.disableConfirm")}
              </Button>
            </View>
          </View>
        )}
      </View>
    </Section>
  );
}

function PlatformIcon({ platform, color }: { platform: common.Platform; color: string }) {
  if (platform === common.Platform.IOS || platform === common.Platform.ANDROID)
    return <Smartphone size={20} color={color} />;
  if (platform === common.Platform.DESKTOP) return <Laptop size={20} color={color} />;
  return <Globe size={20} color={color} />;
}

/** Devices signed in to the account, this one first; any other can be signed out. */
function SessionsSection() {
  const { t } = useTranslation(["settings", "common"]);
  const { colors } = useTheme();
  const query = useQueryClient();
  const { data, isPending } = useQuery({
    queryKey: securityKeys.sessions,
    queryFn: () => api().service(identity.AccountService).listSessions({}),
  });
  const revoke = useMutation({
    mutationFn: async (sessions: identity.Session[]) => {
      const account = currentAccount();
      if (!account) return;
      const service = account.api.service(identity.AccountService);
      await Promise.all(
        sessions.map((session) => service.revokeSession({ sessionId: session.id })),
      );
    },
    onSuccess: (_, sessions) =>
      Alert.alert(
        sessions.length === 1
          ? t("settings:security.sessions.revoked")
          : t("settings:security.sessions.revokedOthers"),
      ),
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "settings" })),
    onSettled: () => query.invalidateQueries({ queryKey: securityKeys.sessions }),
  });
  const confirm = (title: string, sessions: identity.Session[]) =>
    Alert.alert(title, t("settings:security.sessions.revokeBody"), [
      { text: t("common:actions.cancel"), style: "cancel" },
      {
        text: t("settings:security.sessions.revokeConfirm"),
        style: "destructive",
        onPress: () => revoke.mutate(sessions),
      },
    ]);

  const sessions = [...(data?.sessions ?? [])].sort(
    (a, b) =>
      Number(b.current) - Number(a.current) || toMillis(b.lastActiveAt) - toMillis(a.lastActiveAt),
  );
  const others = sessions.filter((session) => !session.current);
  return (
    <Section
      title={t("settings:security.sessions.title")}
      lead={t("settings:security.sessions.lead")}
    >
      {isPending && <Spinner />}
      {sessions.map((session) => {
        const name = session.client?.deviceName || t("settings:security.unknownDevice");
        const place = session.location || session.ip;
        const when = session.current
          ? t("settings:security.sessions.activeNow")
          : compactAge(i18n, toMillis(session.lastActiveAt));
        return (
          <View
            key={session.id}
            testID="session-row"
            className="flex-row items-center gap-3 border-b border-line px-4 py-3"
          >
            <PlatformIcon
              platform={session.client?.platform ?? common.Platform.UNSPECIFIED}
              color={colors.ink}
            />
            <View className="flex-1">
              <Text numberOfLines={1} className="font-bold text-ink">
                {name}
              </Text>
              {(session.addedBySwitch || session.parked) && (
                <Text className="text-xs font-bold text-ink-faint">
                  {[
                    session.addedBySwitch && t("settings:security.sessions.addedBySwitch"),
                    session.parked && t("settings:security.sessions.parked"),
                  ]
                    .filter(Boolean)
                    .join(" ・ ")}
                </Text>
              )}
              <Text className="text-xs text-ink-soft">{place ? `${place} ・ ${when}` : when}</Text>
            </View>
            {session.current ? (
              <View className="rounded-full bg-mint px-3 py-1">
                <Text className="text-xs font-black text-ink">
                  {t("settings:security.sessions.current")}
                </Text>
              </View>
            ) : (
              <Button
                testID={`session-revoke-${session.id}`}
                size="sm"
                variant="secondary"
                disabled={revoke.isPending}
                onPress={() =>
                  confirm(t("settings:security.sessions.revokeTitle", { name }), [session])
                }
              >
                {t("settings:security.sessions.revoke")}
              </Button>
            )}
          </View>
        );
      })}
      {others.length > 1 && (
        <View className="items-start px-2 py-2">
          <Button
            testID="session-revoke-others"
            size="sm"
            variant="ghost"
            disabled={revoke.isPending}
            onPress={() => confirm(t("settings:security.sessions.revokeOthersTitle"), others)}
          >
            {t("settings:security.sessions.revokeOthers")}
          </Button>
        </View>
      )}
    </Section>
  );
}
