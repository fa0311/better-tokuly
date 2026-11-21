import { createFileRoute, Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

export const Route = createFileRoute("/reset-password")({
  validateSearch: (search): { token?: string } => ({ token: typeof search.token === "string" ? search.token : undefined }),
  component: ResetPassword,
});

/** Without a token: ask for the reset mail. With one: choose the new password. */
function ResetPassword() {
  const { t } = useTranslation("auth");
  const { token } = Route.useSearch();
  const [value, setValue] = useState("");
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (token) await authClient().resetPassword({ token, newPassword: value });
      else await authClient().requestPasswordReset({ email: value });
      setDone(true);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  if (done) {
    return (
      <AuthLayout title={token ? t("reset.done") : t("reset.requestDone")}>
        <div className="flex flex-col items-center gap-5 text-center">
          <div className="text-6xl">{token ? "🔑" : "📮"}</div>
          <Link to="/login" className="pressable sticker inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand">
            {t("signIn.submit")}
          </Link>
        </div>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout title={token ? t("reset.title") : t("reset.requestTitle")} lead={token ? undefined : t("reset.requestLead")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        {token ? (
          <TextField label={t("signUp.password")} hint={t("signUp.passwordHint")} type="password" value={value} onChange={(e) => setValue(e.target.value)} autoComplete="new-password" minLength={8} required autoFocus />
        ) : (
          <TextField label={t("signUp.email")} type="email" value={value} onChange={(e) => setValue(e.target.value)} autoComplete="email" required autoFocus />
        )}
        {error && (
          <p role="alert" className="rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold text-danger">
            {error}
          </p>
        )}
        <Button type="submit" size="lg" block loading={busy}>
          {token ? t("reset.submit") : t("reset.requestSubmit")}
        </Button>
      </form>
    </AuthLayout>
  );
}
