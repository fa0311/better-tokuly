import { errorReason, identity, toDate } from "@tokuly/api";
import { isTooYoung } from "@tokuly/core/session";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { ErrorNote, Page } from "#/components/page";
import { TextField } from "#/components/text-field";
import { signOut } from "#/features/session/session";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

/**
 * An account under 13 by the birthdate it gave sees only why it cannot use
 * Tokuly, and the way to delete itself, in place of the app (`children`,
 * background work included). The server lets such a session do nothing else.
 */
export function TooYoungGate({ children }: { children: ReactNode }) {
  const me = useMe();
  if (!me || !isTooYoung(me)) return children;
  return <TooYoung me={me} />;
}

function TooYoung({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [leaving, setLeaving] = useState(false);

  return (
    // No header above it: the gate stands in for the whole stack.
    <SafeAreaView edges={["top"]} className="flex-1 bg-bg">
      <Page testID="too-young-screen">
        <Text className="pt-4 text-center text-6xl" accessibilityElementsHidden>
          🎂
        </Text>
        <Text className="text-2xl font-black text-ink">{t("tooYoung.title")}</Text>
        <Text className="text-sm leading-relaxed text-ink-soft">{t("tooYoung.lead")}</Text>
        <Text className="text-xs text-ink-soft">{t("tooYoung.birthdate")}</Text>
        {me.deletionScheduledAt ? <Scheduled me={me} /> : <Deletion me={me} />}
        <Button
          testID="too-young-sign-out"
          variant="ghost"
          block
          loading={leaving}
          onPress={() => {
            setLeaving(true);
            void signOut();
          }}
        >
          {t("tooYoung.signOut")}
        </Button>
      </Page>
    </SafeAreaView>
  );
}

/** Asks for the password, then schedules the deletion (cancellable for its grace period). */
function Deletion({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [asking, setAsking] = useState(false);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function request() {
    setBusy(true);
    setError(null);
    try {
      const response = await api()
        .service(identity.AccountService)
        .requestAccountDeletion({ currentPassword: password });
      api().session.updateMe({ ...me, deletionScheduledAt: response.deletionScheduledAt });
    } catch (e) {
      setError(
        errorReason(e) === "INVALID_CREDENTIALS"
          ? t("account.password.wrongCurrent")
          : errorMessage(e, { namespace: "settings" }),
      );
      setBusy(false);
    }
  }

  if (!asking) {
    return (
      <View className="gap-3">
        <Text className="text-sm leading-relaxed text-ink">{t("tooYoung.deleteLead")}</Text>
        <Button testID="too-young-delete" variant="danger" block onPress={() => setAsking(true)}>
          {t("data.deletion.start")}
        </Button>
      </View>
    );
  }
  return (
    <View className="gap-3 rounded-3xl bg-surface-2 p-4">
      <Text className="text-lg font-black text-ink">{t("data.deletion.dialogTitle")}</Text>
      {(["grace", "everything", "handle"] as const).map((point) => (
        <Text key={point} className="text-sm leading-relaxed text-ink">
          ・{t(`data.deletion.points.${point}`)}
        </Text>
      ))}
      {me.hasPassword && (
        <TextField
          testID="too-young-password"
          label={t("account.password.current")}
          value={password}
          onChangeText={setPassword}
          secureTextEntry
          autoComplete="current-password"
        />
      )}
      {error && <ErrorNote>{error}</ErrorNote>}
      <Button
        testID="too-young-confirm"
        variant="danger"
        block
        loading={busy}
        disabled={me.hasPassword && !password}
        onPress={request}
      >
        {t("data.deletion.confirm")}
      </Button>
      <Button variant="ghost" block disabled={busy} onPress={() => setAsking(false)}>
        {t("data.deletion.keep")}
      </Button>
    </View>
  );
}

/** When the account will be erased, and the way to keep it after all. */
function Scheduled({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation("settings");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scheduled = toDate(me.deletionScheduledAt);

  async function cancel() {
    setBusy(true);
    setError(null);
    try {
      await api().service(identity.AccountService).cancelAccountDeletion({});
      api().session.updateMe({ ...me, deletionScheduledAt: undefined });
    } catch (e) {
      setError(errorMessage(e, { namespace: "settings" }));
    }
    setBusy(false);
  }

  if (!scheduled) return null;
  const daysLeft = Math.max(0, Math.ceil((scheduled.getTime() - Date.now()) / (24 * 3600 * 1000)));
  return (
    <View className="gap-3">
      <View testID="too-young-scheduled" className="gap-1 rounded-3xl bg-sun p-4">
        <Text className="font-black text-[#17122b]">
          {t("data.deletion.scheduled", {
            date: scheduled.toLocaleDateString(i18n.language, { dateStyle: "long" }),
          })}
        </Text>
        <Text className="text-sm leading-relaxed text-[#17122b]">
          {t("data.deletion.scheduledBody", { count: daysLeft })}
        </Text>
      </View>
      {error && <ErrorNote>{error}</ErrorNote>}
      <Button variant="lime" block loading={busy} onPress={cancel}>
        {t("data.deletion.cancel")}
      </Button>
    </View>
  );
}
