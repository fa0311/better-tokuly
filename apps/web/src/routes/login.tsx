import { startAuthentication } from "@simplewebauthn/browser";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { KeyRound } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { api, authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { parseRedirect } from "#/lib/redirect";

export const Route = createFileRoute("/login")({
  validateSearch: (search): { redirect?: string } => ({
    redirect: parseRedirect(search.redirect),
  }),
  component: Login,
});

function Login() {
  const { t } = useTranslation("auth");
  const navigate = useNavigate();
  const { redirect } = Route.useSearch();
  const [identifier, setIdentifier] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [needsTotp, setNeedsTotp] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"password" | "passkey" | null>(null);

  const finish = () => navigate({ to: redirect ?? "/map", replace: true });

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy("password");
    setError(null);
    try {
      const response = await authClient().signIn({ identifier, password, totpCode });
      if (response.mfaRequired) {
        setNeedsTotp(true);
        return;
      }
      await api().session.adopt(response);
      await finish();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  async function signInWithPasskey() {
    setBusy("passkey");
    setError(null);
    try {
      const begin = await authClient().beginPasskeySignIn({});
      const credential = await startAuthentication({ optionsJSON: JSON.parse(begin.optionsJson) });
      const response = await authClient().finishPasskeySignIn({
        challengeId: begin.challengeId,
        credentialJson: JSON.stringify(credential),
      });
      await api().session.adopt(response);
      await finish();
    } catch (e) {
      // Dismissing the browser's passkey sheet is not an error worth showing.
      if (!(e instanceof Error && e.name === "NotAllowedError")) setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <AuthLayout
      title={needsTotp ? t("signIn.totpTitle") : t("signIn.title")}
      lead={needsTotp ? t("signIn.totpHint") : undefined}
      footer={
        <>
          {t("signIn.noAccount")}{" "}
          {/* Whoever came from a public page comes back to it, whichever way they get an account. */}
          <Link
            to="/signup"
            search={{ redirect }}
            className="font-black text-brand underline-offset-4 hover:underline"
          >
            {t("signIn.toSignUp")}
          </Link>
        </>
      }
    >
      <form onSubmit={submit} className="flex flex-col gap-4">
        {needsTotp ? (
          <TextField
            label={t("signIn.totpCode")}
            value={totpCode}
            onChange={(e) => setTotpCode(e.target.value)}
            inputMode="numeric"
            autoComplete="one-time-code"
            autoFocus
            required
          />
        ) : (
          <>
            <TextField
              label={t("signIn.identifier")}
              value={identifier}
              onChange={(e) => setIdentifier(e.target.value)}
              autoComplete="username webauthn"
              autoCapitalize="none"
              spellCheck={false}
              required
            />
            <TextField
              label={t("signIn.password")}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              required
            />
          </>
        )}
        {error && (
          <p
            role="alert"
            className="rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold text-danger"
          >
            {error}
          </p>
        )}
        <Button
          type="submit"
          size="lg"
          block
          loading={busy === "password"}
          disabled={busy !== null}
        >
          {t("signIn.submit")}
        </Button>
        {!needsTotp && (
          <>
            <Button
              variant="secondary"
              block
              icon={<KeyRound className="size-4" />}
              loading={busy === "passkey"}
              disabled={busy !== null}
              onClick={signInWithPasskey}
            >
              {t("signIn.passkey")}
            </Button>
            <Link
              to="/reset-password"
              className="text-center text-sm font-bold text-ink-soft underline-offset-4 hover:underline"
            >
              {t("signIn.forgot")}
            </Link>
          </>
        )}
      </form>
    </AuthLayout>
  );
}
