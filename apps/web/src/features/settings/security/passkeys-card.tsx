import { browserSupportsWebAuthn, startRegistration } from "@simplewebauthn/browser";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, type identity, toDate } from "@tokuly/api";
import { KeyRound, Plus, Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { ListRow, Skeleton } from "#/components/ui/page";
import { accountClient, authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { PasswordField } from "../account/password-field";
import { keys, refreshMe, usePasskeysAvailable } from "../data";
import { Group } from "../shell/settings-page";

/** A starting name for a new passkey, from what the browser says it runs on. */
function suggestedName(fallback: string): string {
  const agent = navigator.userAgent;
  const system = /iPhone/.test(agent)
    ? "iPhone"
    : /iPad/.test(agent)
      ? "iPad"
      : /Android/.test(agent)
        ? "Android"
        : /Mac OS X/.test(agent)
          ? "Mac"
          : /Windows/.test(agent)
            ? "Windows"
            : /Linux/.test(agent)
              ? "Linux"
              : "";
  return system || fallback;
}

/** Passkeys of the account: list, add one for this device, remove. */
export function PasskeysCard({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation("settings");
  const queryClient = useQueryClient();
  const { data, isPending } = useQuery({
    queryKey: keys.passkeys,
    queryFn: () => accountClient().listPasskeys({}),
  });
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<{ passkey: identity.Passkey | null; open: boolean }>({
    passkey: null,
    open: false,
  });
  const supported = browserSupportsWebAuthn();
  // Passkeys added before the server moved to an IP address stay listed, so
  // they can still be removed.
  const available = usePasskeysAvailable();

  // The account carries the passkey count and whether a password exists.
  const changed = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: keys.passkeys }),
      queryClient.invalidateQueries({ queryKey: keys.securityEvents }),
      refreshMe().catch(() => {}),
    ]);

  const remove = useMutation({
    mutationFn: (passkeyId: string) => accountClient().deletePasskey({ passkeyId }),
    onSuccess: async () => {
      setRemoving((current) => ({ ...current, open: false }));
      toast.success(t("security.passkeys.removed"));
      await changed();
    },
    onError: (error) => {
      setRemoving((current) => ({ ...current, open: false }));
      toast.error(errorMessage(error));
    },
  });

  const date = (timestamp: identity.Passkey["createdAt"]) =>
    toDate(timestamp)?.toLocaleDateString(i18n.language, { dateStyle: "medium" });
  const passkeys = data?.passkeys ?? [];

  return (
    <Group title={t("security.passkeys.title")} lead={t("security.passkeys.lead")} flush>
      {isPending ? (
        <Skeleton className="m-1 h-14" />
      ) : passkeys.length === 0 ? (
        <p className="px-3 py-3 text-sm text-ink-soft">{t("security.passkeys.empty")}</p>
      ) : (
        <ul>
          {passkeys.map((passkey) => (
            <li key={passkey.id}>
              <ListRow
                leading={
                  <span className="grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink bg-sun text-[#17122b]">
                    <KeyRound className="size-5" />
                  </span>
                }
                title={passkey.name}
                subtitle={
                  passkey.lastUsedAt
                    ? t("security.passkeys.addedAndUsed", {
                        added: date(passkey.createdAt),
                        used: date(passkey.lastUsedAt),
                      })
                    : t("security.passkeys.added", { added: date(passkey.createdAt) })
                }
                trailing={
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("security.passkeys.remove", { name: passkey.name })}
                    onClick={() => setRemoving({ passkey, open: true })}
                  >
                    <Trash2 className="size-4" />
                  </Button>
                }
              />
            </li>
          ))}
        </ul>
      )}
      <div className="px-2 pt-2 pb-1">
        {!available ? (
          <p className="px-1 text-sm text-ink-soft">{t("security.passkeys.unavailable")}</p>
        ) : (
          <>
            <Button
              size="sm"
              variant="secondary"
              icon={<Plus className="size-4" />}
              disabled={!supported}
              onClick={() => setAdding(true)}
            >
              {t("security.passkeys.add")}
            </Button>
            {!supported && (
              <p className="mt-2 px-1 text-sm text-ink-soft">
                {t("security.passkeys.unsupported")}
              </p>
            )}
          </>
        )}
      </div>

      {adding && <AddPasskeyDialog me={me} onClose={() => setAdding(false)} onAdded={changed} />}
      <ConfirmDialog
        open={removing.open}
        onOpenChange={(open) => !open && setRemoving((current) => ({ ...current, open: false }))}
        title={t("security.passkeys.removeTitle", { name: removing.passkey?.name ?? "" })}
        description={t("security.passkeys.removeBody")}
        confirmLabel={t("security.passkeys.removeConfirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => removing.passkey && remove.mutate(removing.passkey.id)}
      />
    </Group>
  );
}

function AddPasskeyDialog({
  me,
  onClose,
  onAdded,
}: {
  me: identity.Me;
  onClose(): void;
  onAdded(): Promise<unknown>;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const [name, setName] = useState(() =>
    suggestedName(t("settings:security.passkeys.defaultName")),
  );
  // A new way to sign in takes the password, not just this session.
  const [password, setPassword] = useState("");
  const [passwordError, setPasswordError] = useState<string | null>(null);

  const add = useMutation({
    mutationFn: async () => {
      const begin = await authClient().beginPasskeyRegistration({
        name: name.trim(),
        currentPassword: password,
      });
      const credential = await startRegistration({ optionsJSON: JSON.parse(begin.optionsJson) });
      return authClient().finishPasskeyRegistration({
        challengeId: begin.challengeId,
        credentialJson: JSON.stringify(credential),
      });
    },
    onSuccess: async () => {
      toast.success(t("settings:security.passkeys.addedToast"));
      await onAdded();
      onClose();
    },
    onError: (error) => {
      // Closing the browser's passkey sheet is a decision, not a failure.
      if (
        error instanceof Error &&
        (error.name === "NotAllowedError" || error.name === "AbortError")
      )
        return;
      if (errorReason(error) === "INVALID_CREDENTIALS")
        setPasswordError(t("settings:account.password.wrongCurrent"));
      // The authenticator already holds a passkey for this account.
      else if (error instanceof Error && error.name === "InvalidStateError")
        toast.error(t("errors:PASSKEY_EXISTS"));
      else toast.error(errorMessage(error));
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setPasswordError(null);
    add.mutate();
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && !add.isPending && onClose()}
      title={t("settings:security.passkeys.addTitle")}
      description={t("settings:security.passkeys.addLead")}
    >
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("settings:security.passkeys.name")}
          hint={t("settings:security.passkeys.nameHint")}
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={60}
          autoComplete="off"
          autoFocus
          required
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
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose} disabled={add.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button type="submit" loading={add.isPending} icon={<KeyRound className="size-4" />}>
            {t("settings:security.passkeys.create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
