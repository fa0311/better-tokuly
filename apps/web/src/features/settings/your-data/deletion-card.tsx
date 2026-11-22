import { useMutation } from "@tanstack/react-query";
import { errorReason, type identity, toDate } from "@tokuly/api";
import { CalendarClock } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { PasswordField } from "../account/password-field";
import { adoptMe } from "../data";
import { Group } from "../shell/settings-page";

/**
 * Deleting the account. A request only schedules it: the account is erased
 * after the grace period, and can be kept by cancelling before that.
 */
export function DeletionCard({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation("settings");
  const [asking, setAsking] = useState(false);
  const scheduled = toDate(me.deletionScheduledAt);

  const cancel = useMutation({
    mutationFn: () => accountClient().cancelAccountDeletion({}),
    onSuccess: () => {
      adoptMe({ ...me, deletionScheduledAt: undefined });
      toast.success(t("data.deletion.cancelled"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  if (scheduled) {
    const daysLeft = Math.max(
      0,
      Math.ceil((scheduled.getTime() - Date.now()) / (24 * 3600 * 1000)),
    );
    return (
      <Group title={t("data.deletion.title")}>
        <div
          role="status"
          className="flex items-start gap-3 rounded-2xl bg-sun p-4 text-[#17122b]"
          data-testid="deletion-scheduled"
        >
          <CalendarClock className="mt-0.5 size-6 shrink-0" />
          <div className="min-w-0">
            <p className="font-black">
              {t("data.deletion.scheduled", {
                date: scheduled.toLocaleDateString(i18n.language, { dateStyle: "long" }),
              })}
            </p>
            <p className="mt-1 text-sm leading-relaxed">
              {t("data.deletion.scheduledBody", { count: daysLeft })}
            </p>
          </div>
        </div>
        <Button
          variant="lime"
          className="mt-4"
          loading={cancel.isPending}
          onClick={() => cancel.mutate()}
        >
          {t("data.deletion.cancel")}
        </Button>
      </Group>
    );
  }

  return (
    <Group title={t("data.deletion.title")} lead={t("data.deletion.lead")}>
      <Button variant="danger" size="sm" onClick={() => setAsking(true)}>
        {t("data.deletion.start")}
      </Button>
      {asking && <DeletionDialog me={me} onClose={() => setAsking(false)} />}
    </Group>
  );
}

function DeletionDialog({ me, onClose }: { me: identity.Me; onClose(): void }) {
  const { t } = useTranslation(["settings", "common"]);
  const handle = me.user?.handle ?? "";
  const [reason, setReason] = useState("");
  const [password, setPassword] = useState("");
  const [typedHandle, setTypedHandle] = useState("");
  const [passwordError, setPasswordError] = useState<string | null>(null);

  const request = useMutation({
    mutationFn: () =>
      accountClient().requestAccountDeletion({ currentPassword: password, reason: reason.trim() }),
    onSuccess: (response) => {
      adoptMe({ ...me, deletionScheduledAt: response.deletionScheduledAt });
      toast.success(t("settings:data.deletion.requested"));
      onClose();
    },
    onError: (error) => {
      if (errorReason(error) === "INVALID_CREDENTIALS")
        setPasswordError(t("settings:account.password.wrongCurrent"));
      else toast.error(errorMessage(error));
    },
  });

  // Typing the handle makes sure this is the account the person means to delete.
  const confirmed = typedHandle.trim().replace(/^@/, "").toLowerCase() === handle.toLowerCase();
  const submit = (event: FormEvent) => {
    event.preventDefault();
    setPasswordError(null);
    if (confirmed) request.mutate();
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && !request.isPending && onClose()}
      title={t("settings:data.deletion.dialogTitle")}
      description={t("settings:data.deletion.dialogLead")}
    >
      <form onSubmit={submit} className="flex flex-col gap-4">
        <ul className="flex list-disc flex-col gap-1 rounded-2xl bg-surface-2 py-3 pr-4 pl-8 text-sm leading-relaxed">
          <li>{t("settings:data.deletion.points.grace")}</li>
          <li>{t("settings:data.deletion.points.everything")}</li>
          <li>{t("settings:data.deletion.points.handle")}</li>
        </ul>
        <TextArea
          label={t("settings:data.deletion.reason")}
          placeholder={t("settings:data.deletion.reasonPlaceholder")}
          value={reason}
          onChange={(event) => setReason(event.target.value)}
          maxLength={500}
          rows={2}
        />
        {me.hasPassword && (
          <PasswordField
            label={t("settings:account.password.current")}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            error={passwordError ?? undefined}
            autoComplete="current-password"
            required
          />
        )}
        <TextField
          label={t("settings:data.deletion.typeHandle", { handle })}
          value={typedHandle}
          onChange={(event) => setTypedHandle(event.target.value)}
          placeholder={handle}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          required
        />
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose} disabled={request.isPending}>
            {t("settings:data.deletion.keep")}
          </Button>
          <Button
            type="submit"
            variant="danger"
            loading={request.isPending}
            disabled={!confirmed || (me.hasPassword && !password)}
          >
            {t("settings:data.deletion.confirm")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
