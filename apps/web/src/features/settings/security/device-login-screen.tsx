import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { common, identity, toMillis } from "@tokuly/api";
import { Laptop, MonitorSmartphone, Smartphone } from "lucide-react";
import { type FormEvent, useState } from "react";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { authClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { compactDuration } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { keys } from "../data";
import { SettingsPage } from "../shell/settings-page";

const PLATFORM_KEYS: Record<number, "web" | "ios" | "android" | "desktop"> = {
  [common.Platform.WEB]: "web",
  [common.Platform.IOS]: "ios",
  [common.Platform.ANDROID]: "android",
  [common.Platform.DESKTOP]: "desktop",
};

/**
 * `/settings/security/device`: lets an app without a browser session (Studio,
 * a TV, a command line) sign in. It shows a code; here the signed-in person
 * sees which device is asking and approves or refuses it.
 */
export function DeviceLoginScreen({
  code,
  onCodeChange,
}: {
  /** The code from the address, if any. */ code: string | undefined;
  onCodeChange(code: string | undefined): void;
}) {
  const { t } = useTranslation("settings");
  return (
    <SettingsPage title={t("security.device.title")} back="/settings/security">
      {code ? (
        <DeviceRequest key={code} code={code} onOtherCode={() => onCodeChange(undefined)} />
      ) : (
        <CodeForm onSubmit={onCodeChange} />
      )}
    </SettingsPage>
  );
}

function CodeForm({ onSubmit }: { onSubmit(code: string): void }) {
  const { t } = useTranslation("settings");
  const [value, setValue] = useState("");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (value.trim()) onSubmit(value.trim());
  };
  return (
    <Card>
      <div className="text-5xl">📲</div>
      <h2 className="mt-3 text-xl font-black">{t("security.device.enterTitle")}</h2>
      <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("security.device.enterLead")}</p>
      <form onSubmit={submit} className="mt-5 flex flex-col gap-4">
        <TextField
          label={t("security.device.code")}
          value={value}
          onChange={(event) => setValue(event.target.value.toUpperCase())}
          placeholder="ABCD-2345"
          autoCapitalize="characters"
          autoComplete="off"
          spellCheck={false}
          maxLength={12}
          autoFocus
          required
        />
        <Button type="submit" disabled={!value.trim()}>
          {t("security.device.check")}
        </Button>
      </form>
    </Card>
  );
}

function DeviceRequest({ code, onOtherCode }: { code: string; onOtherCode(): void }) {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const [answer, setAnswer] = useState<"approved" | "denied" | null>(null);
  const request = useQuery({
    queryKey: ["identity", "device-login", code],
    queryFn: () => authClient().getDeviceLogin({ userCode: code }),
    retry: false,
    gcTime: 0,
  });

  const respond = useMutation({
    mutationFn: (approve: boolean) => authClient().approveDeviceLogin({ userCode: code, approve }),
    onSuccess: (_, approve) => {
      setAnswer(approve ? "approved" : "denied");
      // The answer is in the security log, and an approved device is a new session.
      void queryClient.invalidateQueries({ queryKey: keys.securityEvents });
      void queryClient.invalidateQueries({ queryKey: keys.sessions });
    },
  });

  const another = (
    <Button variant="secondary" onClick={onOtherCode}>
      {t("security.device.otherCode")}
    </Button>
  );

  if (answer === "approved")
    return (
      <EmptyState
        emoji="🎉"
        title={t("security.device.approved")}
        body={t("security.device.approvedBody")}
        action={another}
      />
    );
  if (answer === "denied")
    return (
      <EmptyState
        emoji="🙅"
        title={t("security.device.denied")}
        body={t("security.device.deniedBody")}
        action={another}
      />
    );
  if (request.isPending) return <Skeleton className="h-72" />;
  if (request.isError)
    return (
      <EmptyState
        emoji="🤔"
        title={errorMessage(request.error, { namespace: "settings" })}
        body={t("security.device.invalidBody")}
        action={another}
      />
    );

  const { client, status, expiresAt } = request.data;
  if (status !== identity.DeviceLoginStatus.PENDING) {
    const expired = status === identity.DeviceLoginStatus.EXPIRED;
    return (
      <EmptyState
        emoji={expired ? "⌛" : "🔁"}
        title={expired ? t("security.device.expired") : t("security.device.used")}
        body={t("security.device.invalidBody")}
        action={another}
      />
    );
  }

  const platform = PLATFORM_KEYS[client?.platform ?? common.Platform.UNSPECIFIED];
  const DeviceIcon =
    platform === "ios" || platform === "android"
      ? Smartphone
      : platform === "desktop"
        ? Laptop
        : MonitorSmartphone;
  const details = [
    platform ? t(`security.device.platforms.${platform}`) : "",
    client?.appVersion ? `v${client.appVersion}` : "",
  ]
    .filter(Boolean)
    .join(" ・ ");
  const remaining = toMillis(expiresAt) - Date.now();

  return (
    <Card>
      <h2 className="text-xl font-black">{t("security.device.question")}</h2>
      <p className="mt-1 text-sm leading-relaxed text-ink-soft">
        {t("security.device.questionLead")}
      </p>

      <div className="mt-5 flex items-center gap-3 rounded-2xl bg-surface-2 p-4">
        <span className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-[#17122b]">
          <DeviceIcon className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-lg font-black" data-testid="device-name">
            {client?.deviceName || t("security.unknownDevice")}
          </p>
          {details && <p className="truncate text-sm text-ink-soft">{details}</p>}
        </div>
      </div>

      <p className="mt-5 text-center text-sm font-bold text-ink-soft">
        {t("security.device.codeOnDevice")}
      </p>
      <p
        className="mt-1 text-center font-mono text-3xl font-black tracking-[0.2em]"
        data-testid="device-code"
      >
        {code.toUpperCase()}
      </p>
      {remaining > 0 && (
        <p className="mt-1 text-center text-xs font-bold text-ink-faint">
          {t("security.device.expiresIn", { duration: compactDuration(remaining) })}
        </p>
      )}

      {respond.isError && (
        <p
          role="alert"
          className="mt-4 rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold text-danger"
        >
          {errorMessage(respond.error, { namespace: "settings" })}
        </p>
      )}
      <div className="mt-6 grid grid-cols-2 gap-3">
        <Button
          variant="secondary"
          disabled={respond.isPending}
          loading={respond.isPending && respond.variables === false}
          onClick={() => respond.mutate(false)}
        >
          {t("security.device.deny")}
        </Button>
        <Button
          variant="lime"
          disabled={respond.isPending}
          loading={respond.isPending && respond.variables === true}
          onClick={() => respond.mutate(true)}
        >
          {t("security.device.approve")}
        </Button>
      </div>
      <p className="mt-4 text-center text-xs leading-relaxed text-ink-faint">
        {t("security.device.warning")}
      </p>
    </Card>
  );
}
