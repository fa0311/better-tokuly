import { admin } from "@tokuly/api";
import i18n from "#/lib/i18n";

/** What the settings page and its dialogs share. */
export const settingsKey = ["admin", "server-settings"];

/** The name of where a value comes from, as the page shows it. */
export function sourceLabel(source: admin.ServerSettingSource): string {
  switch (source) {
    case admin.ServerSettingSource.CONSOLE:
      return i18n.t("admin:settings.sources.console");
    case admin.ServerSettingSource.FILE:
      return i18n.t("admin:settings.sources.file");
    case admin.ServerSettingSource.ENVIRONMENT:
      return i18n.t("admin:settings.sources.environment");
    case admin.ServerSettingSource.COMMAND_LINE:
      return i18n.t("admin:settings.sources.commandLine");
    default:
      return i18n.t("admin:settings.sources.default");
  }
}

export function processNames(setting: admin.ServerSetting): string {
  const names: Record<admin.ServerProcess, string> = {
    [admin.ServerProcess.UNSPECIFIED]: "",
    [admin.ServerProcess.API]: "api",
    [admin.ServerProcess.REALTIME]: "realtime",
    [admin.ServerProcess.WORKER]: "worker",
  };
  return setting.processes.map((process) => names[process]).join(" / ");
}

/** A value as text: on/off for switches, a placeholder for empty or unset values. */
export function valueText(setting: admin.ServerSetting, value: string | undefined): string {
  if (value === undefined) return i18n.t("admin:settings.unset");
  if (setting.kind === admin.ServerSettingKind.BOOL)
    return i18n.t(value === "true" ? "admin:settings.on" : "admin:settings.off");
  return value === "" ? i18n.t("admin:settings.emptyText") : value;
}

export function secretText(setting: admin.ServerSetting): string {
  if (!setting.secretSet) return i18n.t("admin:settings.secretUnset");
  return setting.secretLast4
    ? i18n.t("admin:settings.secretSetLast4", { last4: setting.secretLast4 })
    : i18n.t("admin:settings.secretSet");
}
