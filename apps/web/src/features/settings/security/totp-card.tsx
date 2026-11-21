import { useMutation, useQuery } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { Copy, Download, ShieldCheck, ShieldOff } from "lucide-react";
import QRCode from "qrcode";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { accountClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { refreshMe } from "../data";
import { Group, StateChip } from "../shell/settings-page";

/** Two-step verification with an authenticator app: turn on (QR, code, recovery codes) and off. */
export function TotpCard({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [dialog, setDialog] = useState<"setup" | "disable" | null>(null);

  return (
    <Group title={t("security.totp.title")} lead={t("security.totp.lead")}>
      <div className="flex items-center gap-3">
        <span className={cn("grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink", me.totpEnabled ? "bg-mint text-[#17122b]" : "bg-surface-3 text-ink")}>
          {me.totpEnabled ? <ShieldCheck className="size-5" /> : <ShieldOff className="size-5" />}
        </span>
        <div className="min-w-0 flex-1">
          <StateChip tone={me.totpEnabled ? "good" : "off"}>{me.totpEnabled ? t("security.totp.on") : t("security.totp.off")}</StateChip>
          <p className="mt-1 text-sm text-ink-soft">{me.totpEnabled ? t("security.totp.onHint") : t("security.totp.offHint")}</p>
        </div>
        {me.totpEnabled ? (
          <Button size="sm" variant="secondary" className="shrink-0" onClick={() => setDialog("disable")}>
            {t("security.totp.disable")}
          </Button>
        ) : (
          <Button size="sm" variant="lime" className="shrink-0" onClick={() => setDialog("setup")}>
            {t("security.totp.enable")}
          </Button>
        )}
      </div>
      {dialog === "setup" && <TotpSetupDialog onClose={() => setDialog(null)} />}
      {dialog === "disable" && <TotpDisableDialog onClose={() => setDialog(null)} />}
    </Group>
  );
}

/** Keeps a code field to what a code can be: digits, optionally spaced. */
const digitsOnly = (value: string) => value.replace(/[^\d\s]/g, "").slice(0, 7);

function TotpSetupDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation(["settings", "common"]);
  const [code, setCode] = useState("");
  const [codeError, setCodeError] = useState<string | null>(null);
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);

  // Starting the setup creates the secret; it stays valid for ten minutes.
  const setup = useQuery({
    queryKey: ["identity", "totp-setup"],
    queryFn: async () => {
      const begun = await accountClient().beginTotpSetup({});
      const qr = await QRCode.toDataURL(begun.otpauthUrl, { margin: 1, width: 440, color: { dark: "#17122b", light: "#ffffff" } });
      return { secret: begun.secret, qr };
    },
    // Never reuse or silently replace a secret: the person may already have scanned it.
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
    retry: false,
    refetchOnReconnect: false,
  });

  const enable = useMutation({
    mutationFn: () => accountClient().enableTotp({ code }),
    onSuccess: async (response) => {
      setRecoveryCodes(response.recoveryCodes);
      await refreshMe().catch(() => {});
    },
    onError: (error) => setCodeError(errorMessage(error)),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setCodeError(null);
    enable.mutate();
  };
  const copySecret = async () => {
    if (!setup.data) return;
    await navigator.clipboard.writeText(setup.data.secret);
    toast.success(t("common:actions.copied"));
  };

  if (recoveryCodes) return <RecoveryCodesDialog codes={recoveryCodes} onClose={onClose} />;

  return (
    <Dialog open onOpenChange={(open) => !open && !enable.isPending && onClose()} title={t("settings:security.totp.setupTitle")} description={t("settings:security.totp.setupLead")}>
      {setup.isError ? (
        <div className="flex flex-col items-center gap-3 py-6 text-center">
          <p className="text-sm font-bold text-danger">{errorMessage(setup.error)}</p>
          <Button variant="secondary" size="sm" onClick={() => setup.refetch()}>
            {t("common:actions.retry")}
          </Button>
        </div>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-5">
          <div>
            <p className="px-1 pb-2 text-sm font-black">{t("settings:security.totp.step1")}</p>
            <div className="sticker mx-auto grid size-52 place-items-center rounded-3xl bg-white p-3">
              {setup.data ? <img src={setup.data.qr} alt={t("settings:security.totp.qrAlt")} className="size-full" /> : <Skeleton className="size-full" />}
            </div>
            <p className="mt-4 px-1 text-sm text-ink-soft">{t("settings:security.totp.manual")}</p>
            <div className="mt-1.5 flex items-center gap-2 rounded-2xl bg-surface-2 py-1.5 pr-1.5 pl-4">
              {setup.data ? (
                <code data-testid="totp-secret" className="min-w-0 flex-1 break-all font-mono text-sm font-bold tracking-wider">
                  {setup.data.secret}
                </code>
              ) : (
                <Skeleton className="h-5 flex-1" />
              )}
              <Button size="icon-sm" variant="ghost" aria-label={t("settings:security.totp.copySecret")} onClick={copySecret} disabled={!setup.data}>
                <Copy className="size-4" />
              </Button>
            </div>
          </div>
          <TextField
            label={t("settings:security.totp.step2")}
            hint={t("settings:security.totp.codeHint")}
            value={code}
            onChange={(event) => setCode(digitsOnly(event.target.value))}
            error={codeError ?? undefined}
            inputMode="numeric"
            autoComplete="one-time-code"
            placeholder="123456"
            required
          />
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose} disabled={enable.isPending}>
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" loading={enable.isPending} disabled={!setup.data || code.replace(/\s/g, "").length !== 6}>
              {t("settings:security.totp.confirm")}
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** Shown once, right after two-step verification was turned on. */
function RecoveryCodesDialog({ codes, onClose }: { codes: string[]; onClose(): void }) {
  const { t } = useTranslation(["settings", "common"]);
  const text = codes.join("\n");
  const copy = async () => {
    await navigator.clipboard.writeText(text);
    toast.success(t("common:actions.copied"));
  };
  const download = () => {
    const url = URL.createObjectURL(new Blob([`${t("settings:security.totp.recoveryFileHeader")}\n\n${text}\n`], { type: "text/plain" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = "tokuly-recovery-codes.txt";
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("settings:security.totp.recoveryTitle")}
      description={t("settings:security.totp.recoveryLead")}
      footer={<Button onClick={onClose}>{t("settings:security.totp.recoveryDone")}</Button>}
    >
      <ul data-testid="recovery-codes" className="grid grid-cols-2 gap-2 rounded-2xl bg-surface-2 p-4 font-mono text-sm font-bold">
        {codes.map((code) => (
          <li key={code} className="text-center tracking-wider">
            {code}
          </li>
        ))}
      </ul>
      <div className="mt-4 flex flex-wrap gap-2">
        <Button size="sm" variant="secondary" icon={<Copy className="size-4" />} onClick={copy}>
          {t("common:actions.copy")}
        </Button>
        <Button size="sm" variant="secondary" icon={<Download className="size-4" />} onClick={download}>
          {t("settings:security.totp.recoveryDownload")}
        </Button>
      </div>
    </Dialog>
  );
}

function TotpDisableDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation(["settings", "common"]);
  const [code, setCode] = useState("");
  const [codeError, setCodeError] = useState<string | null>(null);

  const disable = useMutation({
    mutationFn: () => accountClient().disableTotp({ code: code.trim() }),
    onSuccess: async () => {
      toast.success(t("settings:security.totp.disabled"));
      await refreshMe().catch(() => {});
      onClose();
    },
    onError: (error) => setCodeError(errorMessage(error)),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setCodeError(null);
    disable.mutate();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !disable.isPending && onClose()} title={t("settings:security.totp.disableTitle")} description={t("settings:security.totp.disableLead")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("settings:security.totp.disableCode")}
          hint={t("settings:security.totp.disableCodeHint")}
          value={code}
          onChange={(event) => setCode(event.target.value)}
          error={codeError ?? undefined}
          autoComplete="one-time-code"
          autoCapitalize="none"
          spellCheck={false}
          autoFocus
          required
        />
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose} disabled={disable.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button type="submit" variant="danger" loading={disable.isPending}>
            {t("settings:security.totp.disableConfirm")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
