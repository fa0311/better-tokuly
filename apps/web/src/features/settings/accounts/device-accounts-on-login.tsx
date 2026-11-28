import { useMutation, useQuery } from "@tanstack/react-query";
import { deviceAccountsQuery } from "@tokuly/core/session";
import { type FormEvent, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { EmojiText } from "#/features/emoji";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { enterAccount } from "./accounts-section";

/**
 * On the sign-in screen: the accounts still signed in on this browser after
 * one of them signed out. One tap continues as one of them.
 */
export function DeviceAccountsOnLogin() {
  const { t } = useTranslation("accounts");
  const { data } = useQuery({ ...deviceAccountsQuery(authClient), retry: false });
  const [asking, setAsking] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const switchTo = useMutation({
    mutationFn: (sessionId: string) =>
      authClient().switchAccount({ sessionId, totpCode: asking === sessionId ? code : "" }),
    onSuccess: (response, sessionId) => {
      if (response.mfaRequired) setAsking(sessionId);
      else enterAccount();
    },
    onError: (e) => setError(errorMessage(e, { namespace: "accounts" })),
  });
  const accounts = data?.accounts ?? [];
  if (accounts.length === 0) return null;
  const submitCode = (event: FormEvent) => {
    event.preventDefault();
    if (asking) switchTo.mutate(asking);
  };

  return (
    <section className="mb-6" aria-label={t("onDevice")}>
      <h2 className="px-1 text-sm font-black text-ink-soft">{t("onDevice")}</h2>
      <ul className="mt-2 flex flex-col gap-2">
        {accounts.map((account) => (
          <li key={account.sessionId}>
            <button
              type="button"
              onClick={() => switchTo.mutate(account.sessionId)}
              disabled={switchTo.isPending}
              className="pressable sticker-sm flex w-full items-center gap-3 rounded-2xl bg-surface p-3 text-left"
            >
              <Avatar user={account.user} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-black">
                  <EmojiText text={account.user?.displayName ?? ""} />
                </span>
                <span className="block truncate text-sm font-bold text-ink-soft">
                  @{account.user?.handle}
                </span>
              </span>
            </button>
            {asking === account.sessionId && (
              <form onSubmit={submitCode} className="mt-2 flex items-end gap-2">
                <TextField
                  className="flex-1"
                  label={t("code")}
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  autoFocus
                  required
                />
                <Button type="submit" loading={switchTo.isPending}>
                  {t("codeSubmit")}
                </Button>
              </form>
            )}
          </li>
        ))}
      </ul>
      {error && (
        <p role="alert" className="mt-2 text-sm font-bold text-danger">
          {error}
        </p>
      )}
    </section>
  );
}
