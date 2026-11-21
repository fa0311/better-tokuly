import { useMutation } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { BadgeCheck, MailWarning } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { accountClient, authClient } from "#/lib/api";
import { errorReason } from "@tokuly/api";
import { errorMessage } from "#/lib/errors";
import { Group, StateChip } from "../shell/settings-page";
import { PasswordField } from "./password-field";

/** The sign-in email: whether it is verified, sending the verification mail again, changing it. */
export function EmailCard({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [changing, setChanging] = useState(false);
  // A requested change only applies once the new address is verified.
  const [pendingEmail, setPendingEmail] = useState<string | null>(null);

  const resend = useMutation({
    mutationFn: () => authClient().requestEmailVerification({}),
    onSuccess: () => toast.success(t("account.email.verificationSent", { email: me.email })),
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Group title={t("account.email.title")}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <p className="min-w-0 break-all text-lg font-black leading-snug" data-testid="account-email">
          {me.email}
        </p>
        {me.emailVerified ? (
          <StateChip tone="good" icon={<BadgeCheck />}>
            {t("account.email.verified")}
          </StateChip>
        ) : (
          <StateChip tone="warn" icon={<MailWarning />}>
            {t("account.email.unverified")}
          </StateChip>
        )}
      </div>
      {!me.emailVerified && <p className="mt-2 text-sm leading-relaxed text-ink-soft">{t("account.email.unverifiedLead")}</p>}
      {pendingEmail && pendingEmail !== me.email && (
        <p role="status" className="mt-3 rounded-2xl bg-brand-soft px-4 py-3 text-sm font-bold leading-relaxed">
          {t("account.email.changePending", { email: pendingEmail })}
        </p>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        {!me.emailVerified && (
          <Button size="sm" variant="lime" loading={resend.isPending} onClick={() => resend.mutate()}>
            {t("account.email.resend")}
          </Button>
        )}
        <Button size="sm" variant="secondary" onClick={() => setChanging(true)}>
          {t("account.email.change")}
        </Button>
      </div>
      {changing && (
        <ChangeEmailDialog
          me={me}
          onClose={() => setChanging(false)}
          onRequested={(email) => {
            setPendingEmail(email);
            setChanging(false);
          }}
        />
      )}
    </Group>
  );
}

function ChangeEmailDialog({ me, onClose, onRequested }: { me: identity.Me; onClose(): void; onRequested(email: string): void }) {
  const { t } = useTranslation(["settings", "common"]);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<{ field: "email" | "password"; message: string } | null>(null);

  const change = useMutation({
    mutationFn: () => accountClient().changeEmail({ newEmail: email.trim(), currentPassword: password }),
    onSuccess: () => {
      toast.success(t("settings:account.email.changeSent"));
      onRequested(email.trim().toLowerCase());
    },
    onError: (failure) => {
      const reason = errorReason(failure);
      if (reason === "INVALID_CREDENTIALS") setError({ field: "password", message: t("settings:account.password.wrongCurrent") });
      else if (reason?.startsWith("EMAIL_")) setError({ field: "email", message: errorMessage(failure) });
      else toast.error(errorMessage(failure));
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    change.mutate();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("settings:account.email.changeTitle")} description={t("settings:account.email.changeLead")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("settings:account.email.newEmail")}
          type="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          error={error?.field === "email" ? error.message : undefined}
          autoComplete="email"
          autoFocus
          required
        />
        {me.hasPassword && (
          <PasswordField
            label={t("settings:account.password.current")}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            error={error?.field === "password" ? error.message : undefined}
            autoComplete="current-password"
            required
          />
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose} disabled={change.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button type="submit" loading={change.isPending}>
            {t("settings:account.email.changeSubmit")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
