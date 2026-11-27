import { useQueryClient } from "@tanstack/react-query";
import { asksForBirthdateNow } from "@tokuly/core/session";
import { type FormEvent, type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { adoptMe, keys } from "../data";

/**
 * Asks an account made before sign-up asked for a birthdate for it, before
 * the app; everything else shows `children`. 「あとで」 holds while the account
 * stays signed in (`asksForBirthdateNow`): it is forgotten at sign-out, so
 * the next sign-in asks again.
 */
export function BirthdateGate({ children }: { children: ReactNode }) {
  const me = useMe();
  const userId = me?.user?.id ?? null;
  const [later, setLater] = useState<string | null>(null);
  useEffect(() => {
    if (!userId) setLater(null);
  }, [userId]);
  if (!asksForBirthdateNow(me, later)) return children;
  return <BirthdatePrompt onLater={() => setLater(userId)} />;
}

function BirthdatePrompt({ onLater }: { onLater: () => void }) {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const [birthdate, setBirthdate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const response = await accountClient().setBirthdate({ birthdate });
      if (response.settings) queryClient.setQueryData(keys.settings, response.settings);
      adoptMe(response.me);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <AuthLayout title={t("birthdate.title")} lead={t("birthdate.lead")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("birthdate.label")}
          hint={t("birthdate.hint")}
          type="date"
          value={birthdate}
          onChange={(e) => setBirthdate(e.target.value)}
          autoComplete="bday"
          required
        />
        {error && (
          <p
            role="alert"
            className="rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold text-danger"
          >
            {error}
          </p>
        )}
        <Button type="submit" size="lg" block loading={busy}>
          {t("birthdate.submit")}
        </Button>
        <Button variant="ghost" block onClick={onLater} disabled={busy}>
          {t("birthdate.later")}
        </Button>
        <p className="text-center text-xs text-ink-soft">{t("birthdate.laterHint")}</p>
      </form>
    </AuthLayout>
  );
}
