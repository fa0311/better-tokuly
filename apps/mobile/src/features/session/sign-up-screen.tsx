import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Switch, Text, View } from "react-native";
import { Button } from "#/components/button";
import { ErrorNote, Page } from "#/components/page";
import { TextField } from "#/components/text-field";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { clientInfo } from "#/platform/device";
import { enterAccount } from "./session";

type HandleState =
  | { status: "idle" | "checking" | "available" }
  | { status: "unavailable"; reason: string };

/** Checks handle availability a moment after typing stops. */
function useHandleCheck(handle: string): HandleState {
  const [state, setState] = useState<HandleState>({ status: "idle" });
  useEffect(() => {
    if (handle.length < 3) {
      setState({ status: "idle" });
      return;
    }
    setState({ status: "checking" });
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = await authClient().checkHandle({ handle });
        if (cancelled) return;
        setState(
          result.available
            ? { status: "available" }
            : { status: "unavailable", reason: result.reason },
        );
      } catch {
        if (!cancelled) setState({ status: "idle" });
      }
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [handle]);
  return state;
}

const EMPTY = { displayName: "", handle: "", email: "", password: "", birthdate: "" };

/** A new account (`add`: beside the one in use). `inviteCode` counts the sign-up as an intro. */
export function SignUpScreen({
  add = false,
  inviteCode = "",
}: {
  add?: boolean;
  inviteCode?: string;
}) {
  const { t } = useTranslation("auth");
  const [form, setForm] = useState(EMPTY);
  const [acceptTerms, setAcceptTerms] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const handleState = useHandleCheck(form.handle);
  const field = (key: keyof typeof EMPTY) => ({
    value: form[key],
    onChangeText: (value: string) => setForm((current) => ({ ...current, [key]: value })),
  });

  const handleMessage =
    handleState.status === "available"
      ? t("signUp.handleAvailable")
      : handleState.status === "unavailable"
        ? handleState.reason === "TAKEN"
          ? t("signUp.handleTaken")
          : handleState.reason === "RESERVED"
            ? t("signUp.handleReserved")
            : t("signUp.handleInvalid")
        : t("signUp.handleHint");

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const response = await authClient().signUp({
        ...form,
        email: form.email.trim(),
        handle: form.handle.trim(),
        acceptTerms,
        inviteCode,
        addAccount: add,
        client: await clientInfo(),
      });
      await enterAccount(response);
    } catch (e) {
      setError(errorMessage(e, { namespace: "accounts" }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Page testID="sign-up-screen">
      <Text className="text-2xl font-black text-ink">{t("signUp.title")}</Text>
      <TextField
        testID="sign-up-display-name"
        label={t("signUp.displayName")}
        hint={t("signUp.displayNameHint")}
        autoComplete="name"
        {...field("displayName")}
      />
      <TextField
        testID="sign-up-handle"
        label={t("signUp.handle")}
        hint={handleMessage}
        error={handleState.status === "unavailable" ? handleMessage : undefined}
        autoCapitalize="none"
        autoCorrect={false}
        {...field("handle")}
      />
      <TextField
        testID="sign-up-email"
        label={t("signUp.email")}
        keyboardType="email-address"
        autoCapitalize="none"
        autoComplete="email"
        textContentType="emailAddress"
        {...field("email")}
      />
      <TextField
        testID="sign-up-password"
        label={t("signUp.password")}
        hint={t("signUp.passwordHint")}
        secureTextEntry
        autoComplete="new-password"
        textContentType="newPassword"
        {...field("password")}
      />
      <TextField
        testID="sign-up-birthdate"
        label={t("signUp.birthdate")}
        hint={t("signUp.birthdateHint")}
        placeholder="2009-04-01"
        keyboardType="numbers-and-punctuation"
        autoComplete="birthdate-full"
        {...field("birthdate")}
      />
      <Pressable
        testID="sign-up-terms"
        accessibilityRole="switch"
        accessibilityState={{ checked: acceptTerms }}
        onPress={() => setAcceptTerms(!acceptTerms)}
        className="flex-row items-center gap-3 rounded-control bg-surface px-4 py-3"
      >
        <Switch value={acceptTerms} onValueChange={setAcceptTerms} />
        <View className="flex-1">
          <Text className="text-sm font-bold text-ink">{t("signUp.terms")}</Text>
        </View>
      </Pressable>
      {error && <ErrorNote>{error}</ErrorNote>}
      <Button
        testID="sign-up-submit"
        size="lg"
        block
        loading={busy}
        disabled={!acceptTerms || handleState.status === "unavailable"}
        onPress={submit}
      >
        {t("signUp.submit")}
      </Button>
    </Page>
  );
}
