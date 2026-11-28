import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { admin } from "@tokuly/api";
import { Download, ExternalLink, History, Lock, Pencil } from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useTranslation } from "#/lib/use-translation";
import { serverSettingsClient } from "../api";
import { ConsolePage, Section } from "../ui/console-page";
import { SearchBox } from "../ui/filters";
import { useSearchText } from "../ui/hooks";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { type Tone, ToneChip } from "../ui/tone";
import { processNames, secretText, settingsKey, sourceLabel, valueText } from "./labels";
import { SettingDialog, SettingHistoryDialog } from "./setting-dialog";
import { FILTERS, type Filter, type SettingsSearch } from "./settings-search";

/** Pages of the console that edit settings of their own; this page links to them. */
const RELATED = [
  { to: "/admin/chat-ai", label: "chat:aiAdmin.nav" },
  { to: "/admin/flags", label: "admin:nav.flags" },
];

const SOURCE_TONE: Record<admin.ServerSettingSource, Tone> = {
  [admin.ServerSettingSource.UNSPECIFIED]: "neutral",
  [admin.ServerSettingSource.DEFAULT]: "neutral",
  [admin.ServerSettingSource.CONSOLE]: "brand",
  [admin.ServerSettingSource.FILE]: "info",
  [admin.ServerSettingSource.ENVIRONMENT]: "warn",
  [admin.ServerSettingSource.COMMAND_LINE]: "warn",
};

const editable = (setting: admin.ServerSetting) => setting.lock === admin.ServerSettingLock.NONE;

function matches(setting: admin.ServerSetting, query: string | undefined, show: Filter): boolean {
  if (show === "console" && setting.consoleValue === undefined && !setting.consoleSecretSet)
    return false;
  if (show === "locked" && editable(setting)) return false;
  if (show === "pending" && !setting.restartPending) return false;
  if (show === "rejected" && !setting.consoleRejected) return false;
  if (!query) return true;
  const needle = query.toLowerCase();
  return [
    setting.section,
    setting.key,
    setting.env,
    setting.description,
    sectionName(setting.section),
  ]
    .join(" ")
    .toLowerCase()
    .includes(needle);
}

const sectionName = (section: string) =>
  i18n.exists(`admin:settings.sections.${section}`)
    ? i18n.t(`admin:settings.sections.${section}`)
    : section;

/**
 * `/admin/settings`: every setting of the backend by section, with its value,
 * default, description and where the value comes from. What the console may
 * change opens in a dialog; the rest says why it is locked.
 */
export function ServerSettingsScreen({ search }: { search: SettingsSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const show = search.show ?? "all";
  const typed = useSearchText(search.q, (q) =>
    navigate({ to: "/admin/settings", search: { ...search, q }, replace: true }),
  );
  const [editing, setEditing] = useState<{ setting: admin.ServerSetting; session: number }>();
  const [history, setHistory] = useState<admin.ServerSetting>();

  const listing = useQuery({
    queryKey: settingsKey,
    queryFn: () => serverSettingsClient().listServerSettings({}),
  });

  const exportFile = useMutation({
    mutationFn: () => serverSettingsClient().exportServerSettings({}),
    onSuccess: ({ toml }) => {
      const url = URL.createObjectURL(new Blob([toml], { type: "application/toml" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = "tokuly.toml";
      link.click();
      URL.revokeObjectURL(url);
      toast.success(t("settings.exported"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "admin" })),
  });

  const sections = useMemo(() => {
    const groups = new Map<string, admin.ServerSetting[]>();
    for (const setting of listing.data?.settings ?? []) {
      if (!matches(setting, typed.query, show)) continue;
      const group = groups.get(setting.section) ?? [];
      group.push(setting);
      groups.set(setting.section, group);
    }
    return [...groups.entries()];
  }, [listing.data, typed.query, show]);
  // Opened when looking for something; otherwise each section starts folded.
  const unfolded = Boolean(typed.query) || show !== "all";

  return (
    <ConsolePage
      title={t("settings.title")}
      actions={
        <Button
          size="sm"
          variant="secondary"
          icon={<Download className="size-4" />}
          loading={exportFile.isPending}
          onClick={() => exportFile.mutate()}
        >
          {t("settings.export")}
        </Button>
      }
    >
      <div className="flex min-w-0 flex-col gap-3">
        <Section>
          <div className="flex flex-col gap-2 px-4 py-3 text-sm">
            <p>{t("settings.intro")}</p>
            {listing.data && (
              <p className="text-xs text-ink-soft">
                {listing.data.filePath
                  ? t("settings.file", { path: listing.data.filePath })
                  : t("settings.noFile")}
              </p>
            )}
            <p className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-bold text-ink-soft">{t("settings.related")}</span>
              {RELATED.map((page) => (
                <Link key={page.to} to={page.to} className="font-bold text-brand underline">
                  {t(page.label)}
                </Link>
              ))}
            </p>
          </div>
        </Section>
        <SearchBox
          label={t("settings.search")}
          placeholder={t("settings.search")}
          value={typed.text}
          onChange={typed.setText}
          onCommit={typed.commit}
        />
        <Tabs<Filter>
          value={show}
          onChange={(next) =>
            navigate({
              to: "/admin/settings",
              search: { ...search, q: typed.query, show: next === "all" ? undefined : next },
              replace: true,
            })
          }
          items={FILTERS.map((value) => ({ value, label: t(`settings.filters.${value}`) }))}
        />
        {listing.isPending ? (
          <RowsSkeleton />
        ) : listing.isError ? (
          <ErrorState error={listing.error} onRetry={() => listing.refetch()} />
        ) : sections.length === 0 ? (
          <EmptyState emoji="🔧" title={t("settings.empty")} />
        ) : (
          sections.map(([section, settings]) => (
            <details
              key={`${section}-${unfolded}`}
              open={unfolded}
              className="group rounded-2xl border-2 border-line bg-surface"
            >
              <summary className="flex min-h-11 cursor-pointer list-none items-center gap-2 px-4 py-2">
                <h2 className="min-w-0 flex-1 truncate text-sm font-black">
                  {sectionName(section)}{" "}
                  <code className="font-mono text-ink-faint">[{section}]</code>
                </h2>
                <span className="text-xs font-bold text-ink-soft">
                  {t("settings.count", { count: settings.length })}
                </span>
              </summary>
              <ul aria-label={sectionName(section)}>
                {settings.map((setting) => (
                  <SettingRow
                    key={setting.key}
                    setting={setting}
                    onEdit={() =>
                      setEditing((current) => ({ setting, session: (current?.session ?? 0) + 1 }))
                    }
                    onHistory={() => setHistory(setting)}
                  />
                ))}
              </ul>
            </details>
          ))
        )}
      </div>
      {editing && (
        <SettingDialog
          key={editing.session}
          setting={editing.setting}
          open
          onOpenChange={(open) => !open && setEditing(undefined)}
        />
      )}
      {history && (
        <SettingHistoryDialog
          setting={history}
          open
          onOpenChange={(open) => !open && setHistory(undefined)}
        />
      )}
    </ConsolePage>
  );
}

function SettingRow({
  setting,
  onEdit,
  onHistory,
}: {
  setting: admin.ServerSetting;
  onEdit(): void;
  onHistory(): void;
}) {
  const { t } = useTranslation("admin");
  const name = `${setting.section}.${setting.key}`;
  const lockText =
    setting.lock === admin.ServerSettingLock.FILE_ONLY
      ? t("settings.lock.fileOnly")
      : setting.lock === admin.ServerSettingLock.OVERRIDDEN
        ? t("settings.lock.overridden", { source: sourceLabel(setting.source) })
        : setting.lock === admin.ServerSettingLock.MANAGED_ELSEWHERE
          ? t("settings.lock.managed")
          : undefined;
  const shadowed =
    setting.lock !== admin.ServerSettingLock.NONE &&
    setting.source !== admin.ServerSettingSource.CONSOLE &&
    (setting.consoleValue !== undefined || setting.consoleSecretSet);
  return (
    <li aria-label={name} className="flex flex-col gap-2 border-t-2 border-line px-4 py-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <code className="min-w-0 font-mono font-bold [overflow-wrap:anywhere]">{setting.key}</code>
        <ToneChip tone={SOURCE_TONE[setting.source]}>{sourceLabel(setting.source)}</ToneChip>
        {setting.secret && <ToneChip icon={<Lock />}>{t("settings.secret")}</ToneChip>}
        {setting.restartPending ? (
          <ToneChip tone="warn">
            {t("settings.restartPending", { processes: processNames(setting) })}
          </ToneChip>
        ) : setting.live ? (
          <ToneChip tone="good">{t("settings.live")}</ToneChip>
        ) : null}
      </div>
      {setting.description && <p className="text-xs text-ink-soft">{setting.description}</p>}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
        <dt className="font-bold text-ink-faint">{t("settings.value")}</dt>
        <dd className="font-mono font-bold [overflow-wrap:anywhere]">
          {setting.secret ? secretText(setting) : valueText(setting, setting.value)}
        </dd>
        {!setting.secret && (
          <>
            <dt className="font-bold text-ink-faint">{t("settings.defaultValue")}</dt>
            <dd className="font-mono [overflow-wrap:anywhere]">
              {valueText(setting, setting.defaultValue)}
            </dd>
          </>
        )}
        <dt className="font-bold text-ink-faint">{t("settings.env")}</dt>
        <dd className="font-mono">{setting.env}</dd>
      </dl>
      {!setting.restartPending && !setting.live && editable(setting) && (
        <p className="text-xs text-ink-faint">
          {t("settings.restart", { processes: processNames(setting) })}
        </p>
      )}
      {lockText && (
        <p className="flex items-center gap-1.5 text-xs font-bold text-ink-soft">
          <Lock className="size-3.5 shrink-0" />
          {lockText}
        </p>
      )}
      {setting.consoleRejected && (
        <p role="alert" className="text-xs font-bold text-danger">
          {t("settings.rejected", { reason: setting.consoleRejected })}
        </p>
      )}
      {shadowed && !setting.consoleRejected && (
        <p className="text-xs text-ink-faint">
          {setting.secret
            ? t("settings.shadowedSecret")
            : t("settings.shadowed", { value: valueText(setting, setting.consoleValue) })}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {setting.lock === admin.ServerSettingLock.MANAGED_ELSEWHERE && setting.managedAt ? (
          <Link
            to={setting.managedAt}
            className="inline-flex items-center gap-1 text-xs font-bold text-brand underline"
          >
            <ExternalLink className="size-3.5" />
            {t("settings.openManaged")}
          </Link>
        ) : (
          editable(setting) && (
            <Button
              size="sm"
              variant="secondary"
              icon={<Pencil className="size-4" />}
              aria-label={t("settings.edit", { key: name })}
              onClick={onEdit}
            >
              {t("settings.editTitle", { key: setting.key })}
            </Button>
          )
        )}
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("settings.history", { key: name })}
          onClick={onHistory}
        >
          <History className="size-4" />
        </Button>
      </div>
    </li>
  );
}
