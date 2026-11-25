import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Text } from "react-native";
import { Button } from "#/components/button";
import { ErrorNote, Page } from "#/components/page";
import { TextField } from "#/components/text-field";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { clientInfo } from "#/platform/device";
import { enterAccount } from "./session";

/**
 * Sign-in with a handle or e-mail and the password, then the second factor
 * when the account has one. `add`: beside the account in use (it is parked).
 */
export function SignInScreen({ add = false }: { add?: boolean }) {
  const { t } = useTranslation("auth");
  const { t: ta } = useTranslation("accounts");
  const [identifier, setIdentifier] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [needsTotp, setNeedsTotp] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const response = await authClient().signIn({
        identifier: identifier.trim(),
        password,
        totpCode: totpCode.trim(),
        addAccount: add,
        client: await clientInfo(),
      });
      if (response.mfaRequired) {
        setNeedsTotp(true);
        return;
      }
      await enterAccount(response);
    } catch (e) {
      setError(errorMessage(e, { namespace: "accounts" }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Page testID="sign-in-screen">
      <Text className="text-2xl font-black text-ink">
        {needsTotp ? t("signIn.totpTitle") : add ? ta("addTitle") : t("signIn.title")}
      </Text>
      {(needsTotp || add) && (
        <Text className="text-sm text-ink-soft">
          {needsTotp ? t("signIn.totpHint") : ta("addLead")}
        </Text>
      )}
      {needsTotp ? (
        <TextField
          testID="sign-in-totp"
          label={t("signIn.totpCode")}
          value={totpCode}
          onChangeText={setTotpCode}
          keyboardType="number-pad"
          textContentType="oneTimeCode"
          autoComplete="one-time-code"
          autoFocus
        />
      ) : (
        <>
          <TextField
            testID="sign-in-identifier"
            label={t("signIn.identifier")}
            value={identifier}
            onChangeText={setIdentifier}
            autoCapitalize="none"
            autoCorrect={false}
            autoComplete="username"
            textContentType="username"
          />
          <TextField
            testID="sign-in-password"
            label={t("signIn.password")}
            value={password}
            onChangeText={setPassword}
            secureTextEntry
            autoComplete="current-password"
            textContentType="password"
            onSubmitEditing={submit}
          />
        </>
      )}
      {error && <ErrorNote>{error}</ErrorNote>}
      <Button
        testID="sign-in-submit"
        size="lg"
        block
        loading={busy}
        disabled={needsTotp ? !totpCode.trim() : !identifier.trim() || !password}
        onPress={submit}
      >
        {t("signIn.submit")}
      </Button>
    </Page>
  );
}
