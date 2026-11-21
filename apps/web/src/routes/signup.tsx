import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { Check, X } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { api, authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

export const Route = createFileRoute("/signup")({
  validateSearch: (search): { invite?: string } => ({
    invite: typeof search.invite === "string" ? search.invite : undefined,
  }),
  component: SignUp,
});

type HandleState = { status: "idle" | "checking" | "available" } | { status: "unavailable"; reason: string };

/** Checks handle availability a moment after the user stops typing. */
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
        setState(result.available ? { status: "available" } : { status: "unavailable", reason: result.reason });
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

function SignUp() {
  const { t } = useTranslation("auth");
  const navigate = useNavigate();
  const { invite } = Route.useSearch();
  const [form, setForm] = useState({ displayName: "", handle: "", email: "", password: "", birthdate: "" });
  const [acceptTerms, setAcceptTerms] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const handleState = useHandleCheck(form.handle);

  const update = (key: keyof typeof form) => (event: { target: { value: string } }) =>
    setForm((current) => ({ ...current, [key]: event.target.value }));

  const handleMessage =
    handleState.status === "available"
      ? t("signUp.handleAvailable")
      : handleState.status === "unavailable"
        ? handleState.reason === "TAKEN"
          ? t("signUp.handleTaken")
          : handleState.reason === "RESERVED"
            ? t("signUp.handleReserved")
            : t("signUp.handleInvalid")
        : undefined;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const response = await authClient().signUp({
        ...form,
        acceptTerms,
        inviteCode: invite ?? "",
        client: { locale: navigator.language, timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone },
      });
      await api().session.adopt(response);
      await navigate({ to: "/map", replace: true });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthLayout
      title={t("signUp.title")}
      footer={
        <>
          {t("signUp.haveAccount")}{" "}
          <Link to="/login" className="font-black text-brand underline-offset-4 hover:underline">
            {t("signUp.toSignIn")}
          </Link>
        </>
      }
    >
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField label={t("signUp.displayName")} hint={t("signUp.displayNameHint")} value={form.displayName} onChange={update("displayName")} autoComplete="nickname" maxLength={40} required />
        <TextField
          label={t("signUp.handle")}
          value={form.handle}
          onChange={(e) => setForm((c) => ({ ...c, handle: e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, "") }))}
          autoComplete="username"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={24}
          required
          hint={handleState.status === "available" ? <span className="font-bold text-success">{handleMessage}</span> : t("signUp.handleHint")}
          error={handleState.status === "unavailable" ? handleMessage : undefined}
          trailing={
            handleState.status === "available" ? (
              <Check className="mr-2 size-5 text-success" aria-hidden />
            ) : handleState.status === "unavailable" ? (
              <X className="mr-2 size-5 text-danger" aria-hidden />
            ) : null
          }
        />
        <TextField label={t("signUp.email")} type="email" value={form.email} onChange={update("email")} autoComplete="email" required />
        <TextField label={t("signUp.password")} hint={t("signUp.passwordHint")} type="password" value={form.password} onChange={update("password")} autoComplete="new-password" minLength={8} required />
        <TextField label={t("signUp.birthdate")} hint={t("signUp.birthdateHint")} type="date" value={form.birthdate} onChange={update("birthdate")} autoComplete="bday" required />
        <label className="flex items-start gap-3 rounded-2xl bg-surface-2 p-3 text-sm font-medium">
          <input type="checkbox" checked={acceptTerms} onChange={(e) => setAcceptTerms(e.target.checked)} className="mt-0.5 size-5 accent-[var(--brand)]" required />
          <span>{t("signUp.terms")}</span>
        </label>
        {error && (
          <p role="alert" className="rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold text-danger">
            {error}
          </p>
        )}
        <Button type="submit" size="lg" block loading={busy} disabled={handleState.status === "unavailable"}>
          {t("signUp.submit")}
        </Button>
      </form>
    </AuthLayout>
  );
}
