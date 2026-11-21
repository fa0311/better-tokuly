import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorReason, type identity } from "@tokuly/api";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { keys, refreshMe } from "../data";
import { Group } from "../shell/settings-page";
import { PasswordField } from "./password-field";

/** Changes the password, or sets the first one for an account that only has passkeys. */
export function PasswordCard({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [error, setError] = useState<{ field: "current" | "next"; message: string } | null>(null);
  const hadPassword = me.hasPassword;

  const change = useMutation({
    mutationFn: () => accountClient().changePassword({ currentPassword: hadPassword ? current : "", newPassword: next }),
    onSuccess: async () => {
      setCurrent("");
      setNext("");
      toast.success(hadPassword ? t("account.password.changed") : t("account.password.created"));
      // The server signed every other device out.
      void queryClient.invalidateQueries({ queryKey: keys.sessions });
      await refreshMe().catch(() => {});
    },
    onError: (failure) => {
      const reason = errorReason(failure);
      if (reason === "INVALID_CREDENTIALS") setError({ field: "current", message: t("account.password.wrongCurrent") });
      else if (reason?.startsWith("PASSWORD_")) setError({ field: "next", message: errorMessage(failure) });
      else toast.error(errorMessage(failure));
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    change.mutate();
  };

  return (
    <Group title={t("account.password.title")} lead={hadPassword ? t("account.password.lead") : t("account.password.leadNone")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        {/* Lets password managers file the new password under the right account. */}
        <input type="text" name="username" autoComplete="username" value={me.user?.handle ?? ""} readOnly hidden />
        {hadPassword && (
          <PasswordField
            label={t("account.password.current")}
            value={current}
            onChange={(event) => setCurrent(event.target.value)}
            error={error?.field === "current" ? error.message : undefined}
            autoComplete="current-password"
            required
          />
        )}
        <PasswordField
          label={t("account.password.new")}
          hint={t("account.password.newHint")}
          value={next}
          onChange={(event) => setNext(event.target.value)}
          error={error?.field === "next" ? error.message : undefined}
          autoComplete="new-password"
          minLength={8}
          required
        />
        <Button type="submit" className="self-end" loading={change.isPending} disabled={!next || (hadPassword && !current)}>
          {hadPassword ? t("account.password.submit") : t("account.password.submitNone")}
        </Button>
      </form>
    </Group>
  );
}
