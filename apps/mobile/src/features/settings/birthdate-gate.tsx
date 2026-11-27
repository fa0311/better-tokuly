import { useQueryClient } from "@tanstack/react-query";
import { identity } from "@tokuly/api";
import { asksForBirthdate } from "@tokuly/core/session";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Text } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { create } from "zustand";
import { Button } from "#/components/button";
import { ErrorNote, Page } from "#/components/page";
import { TextField } from "#/components/text-field";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { settingsKeys } from "./data";

/** Accounts that answered "later" since the app started: the next start asks again. */
const useLater = create<{ users: string[] }>(() => ({ users: [] }));

/**
 * Asks an account made before sign-up asked for a birthdate
 * (`asksForBirthdate`) for it, in place of the screens (`children`).
 */
export function BirthdateGate({ children }: { children: ReactNode }) {
  const me = useMe();
  const userId = me?.user?.id ?? "";
  const later = useLater((s) => s.users.includes(userId));
  if (!me || !asksForBirthdate(me) || later) return children;
  return (
    <BirthdatePrompt onLater={() => useLater.setState((s) => ({ users: [...s.users, userId] }))} />
  );
}

function BirthdatePrompt({ onLater }: { onLater: () => void }) {
  const { t } = useTranslation("settings");
  const query = useQueryClient();
  const [birthdate, setBirthdate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const response = await api().service(identity.AccountService).setBirthdate({ birthdate });
      if (response.settings) query.setQueryData(settingsKeys.settings, response.settings);
      if (response.me) api().session.updateMe(response.me);
    } catch (e) {
      setError(errorMessage(e, { namespace: "settings" }));
      setBusy(false);
    }
  }

  return (
    // No header above it: the gate stands in for the whole stack.
    <SafeAreaView edges={["top"]} className="flex-1 bg-bg">
      <Page testID="birthdate-screen">
        <Text className="pt-4 text-2xl font-black text-ink">{t("birthdate.title")}</Text>
        <Text className="text-sm leading-relaxed text-ink-soft">{t("birthdate.lead")}</Text>
        <TextField
          testID="birthdate-input"
          label={t("birthdate.label")}
          hint={t("birthdate.hint")}
          placeholder="2009-04-01"
          keyboardType="numbers-and-punctuation"
          autoComplete="birthdate-full"
          value={birthdate}
          onChangeText={setBirthdate}
        />
        {error && <ErrorNote>{error}</ErrorNote>}
        <Button testID="birthdate-submit" size="lg" block loading={busy} onPress={submit}>
          {t("birthdate.submit")}
        </Button>
        <Button testID="birthdate-later" variant="ghost" block disabled={busy} onPress={onLater}>
          {t("birthdate.later")}
        </Button>
        <Text className="text-center text-xs text-ink-soft">{t("birthdate.laterHint")}</Text>
      </Page>
    </SafeAreaView>
  );
}
