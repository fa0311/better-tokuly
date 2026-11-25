import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { admin, errorReason, isApiError } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { serverSettingsClient } from "../api";
import { formatDateTime } from "../format";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { processNames, settingsKey, valueText } from "./labels";

/** The text the server says is wrong with a value, under the localized reason. */
function errorText(error: unknown): string {
  const message = errorMessage(error, { namespace: "admin" });
  if (errorReason(error) === "SETTING_INVALID" && isApiError(error) && error.rawMessage)
    return `${message}: ${error.rawMessage}`;
  return message;
}

/**
 * Changes one setting, or takes the console's value away again. Mount it
 * anew for every opening so it starts from the setting as listed.
 */
export function SettingDialog({
  setting,
  open,
  onOpenChange,
}: {
  setting: admin.ServerSetting;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("admin");
  const queryClient = useQueryClient();
  const [value, setValue] = useState(
    setting.secret ? "" : (setting.consoleValue ?? setting.value ?? setting.defaultValue ?? ""),
  );
  const [error, setError] = useState<string>();

  const save = useMutation({
    mutationFn: (reset: boolean) =>
      serverSettingsClient().updateServerSetting({
        section: setting.section,
        key: setting.key,
        value,
        reset,
      }),
    onSuccess: ({ setting: saved }, reset) => {
      toast.success(
        reset
          ? t("settings.resetDone")
          : saved?.restartPending
            ? t("settings.savedRestart")
            : t("settings.saved"),
      );
      onOpenChange(false);
      return queryClient.invalidateQueries({ queryKey: settingsKey });
    },
    onError: (failure) => setError(errorText(failure)),
  });

  const label = setting.secret ? t("settings.newSecret") : t("settings.newValue");
  const input =
    setting.kind === admin.ServerSettingKind.BOOL ? (
      <SwitchRow
        label={setting.key}
        checked={value === "true"}
        onChange={(on) => setValue(String(on))}
      />
    ) : setting.choices.length > 0 ? (
      <SelectField
        label={label}
        value={value}
        onChange={setValue}
        options={setting.choices.map((choice) => ({ value: choice, label: choice }))}
      />
    ) : setting.kind === admin.ServerSettingKind.LIST ? (
      <TextArea
        label={label}
        hint={t("settings.listHint")}
        value={value}
        onChange={(event) => setValue(event.target.value)}
      />
    ) : (
      <TextField
        label={label}
        type={setting.secret ? "password" : "text"}
        autoComplete="off"
        inputMode={
          setting.kind === admin.ServerSettingKind.INTEGER ||
          setting.kind === admin.ServerSettingKind.NUMBER
            ? "decimal"
            : undefined
        }
        hint={
          setting.secret
            ? t("settings.secretHint")
            : setting.kind === admin.ServerSettingKind.INTEGER ||
                setting.kind === admin.ServerSettingKind.NUMBER
              ? t("settings.numberHint")
              : undefined
        }
        value={value}
        onChange={(event) => setValue(event.target.value)}
      />
    );
  const saved = setting.consoleValue !== undefined || setting.consoleSecretSet;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("settings.editTitle", { key: `${setting.section}.${setting.key}` })}
      description={setting.description}
      footer={
        <>
          {saved && (
            <Button
              variant="secondary"
              loading={save.isPending && save.variables}
              onClick={() => save.mutate(true)}
            >
              {t("settings.reset")}
            </Button>
          )}
          <Button loading={save.isPending && !save.variables} onClick={() => save.mutate(false)}>
            {t("settings.save")}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate(false);
        }}
      >
        {input}
        <p className="text-xs text-ink-soft">
          {setting.live
            ? t("settings.live")
            : t("settings.restart", { processes: processNames(setting) })}
          {!setting.secret &&
            ` · ${t("settings.defaultValue")}: ${valueText(setting, setting.defaultValue)}`}
        </p>
        {error && (
          <p role="alert" className="text-sm font-bold text-danger">
            {error}
          </p>
        )}
      </form>
    </Dialog>
  );
}

/** What the console changed about one setting, newest first. */
export function SettingHistoryDialog({
  setting,
  open,
  onOpenChange,
}: {
  setting: admin.ServerSetting;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("admin");
  const changes = useQuery({
    queryKey: [...settingsKey, "changes", setting.section, setting.key],
    queryFn: () =>
      serverSettingsClient().listServerSettingChanges({
        section: setting.section,
        key: setting.key,
        page: { limit: 50 },
      }),
  });
  const shown = (value: string | undefined) =>
    value === undefined ? t("settings.historyNone") : valueText(setting, value);
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("settings.historyTitle", { key: `${setting.section}.${setting.key}` })}
    >
      {changes.isPending ? (
        <RowsSkeleton />
      ) : changes.isError ? (
        <ErrorState error={changes.error} onRetry={() => changes.refetch()} />
      ) : changes.data.changes.length === 0 ? (
        <p className="py-4 text-sm text-ink-soft">{t("settings.historyEmpty")}</p>
      ) : (
        <ol className="flex flex-col gap-2">
          {changes.data.changes.map((change) => (
            <li
              key={change.id}
              className="flex items-start gap-3 rounded-2xl bg-surface-2 px-3 py-2 text-sm"
            >
              {change.actor && <Avatar user={change.actor} size="xs" />}
              <span className="min-w-0 flex-1">
                <span className="block font-mono font-bold [overflow-wrap:anywhere]">
                  {shown(change.oldValue)} → {shown(change.newValue)}
                </span>
                <span className="block text-xs text-ink-soft">
                  {change.actor && t("settings.historyBy", { name: change.actor.displayName })}
                  {" · "}
                  {formatDateTime(change.changedAt)}
                </span>
              </span>
            </li>
          ))}
        </ol>
      )}
    </Dialog>
  );
}
