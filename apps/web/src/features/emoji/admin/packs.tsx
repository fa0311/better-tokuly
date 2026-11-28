import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { emoji as emojiApi, media, toDate } from "@tokuly/api";
import { emojiKeys } from "@tokuly/core/emoji";
import { Download, FileArchive, Upload } from "lucide-react";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { emojiAdminClient, useCategories } from "./shared";

/** Select values of the export's scope (category names never start with a control character). */
const WHOLE = "\u0001whole";
const NONE = "\u0001none";

function formatBytes(bytes: bigint): string {
  const value = Number(bytes);
  return value >= 1024 * 1024
    ? `${(value / 1024 / 1024).toFixed(1)} MB`
    : `${Math.max(1, Math.round(value / 1024))} KB`;
}

function useDateTime() {
  const { i18n } = useTranslation();
  const format = new Intl.DateTimeFormat(i18n.language, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  return (timestamp: Parameters<typeof toDate>[0]) => {
    const date = toDate(timestamp);
    return date ? format.format(date) : "";
  };
}

/** Whole sets in and out: import a ZIP in Misskey's export format, export the catalogue as one. */
export function Packs() {
  const [openImport, setOpenImport] = useState<string>();
  return (
    <div className="grid items-start gap-4 xl:grid-cols-2">
      <div className="flex flex-col gap-4">
        <ImportForm onStarted={setOpenImport} />
        {openImport && <ImportReport key={openImport} importId={openImport} />}
        <ImportHistory current={openImport} onOpen={setOpenImport} />
      </div>
      <Exports />
    </div>
  );
}

// ───────────────────────────── Import ─────────────────────────────

function ImportForm({ onStarted }: { onStarted(importId: string): void }) {
  const { t } = useTranslation("emoji");
  const queryClient = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File>();
  const [policy, setPolicy] = useState<"skip" | "rename">("skip");
  const [category, setCategory] = useState("");
  const [progress, setProgress] = useState(0);

  const start = useMutation({
    mutationFn: async (pack: File) => {
      setProgress(0);
      const asset = await uploadAndProcess(pack, {
        purpose: media.AssetPurpose.CHAT_FILE,
        onProgress: setProgress,
      });
      return emojiAdminClient().startImport({
        assetId: asset.id,
        onConflict:
          policy === "rename"
            ? emojiApi.ImportConflictPolicy.RENAME
            : emojiApi.ImportConflictPolicy.SKIP,
        category: category.trim(),
      });
    },
    onSuccess: async (response) => {
      setFile(undefined);
      toast.success(t("admin.importStarted"));
      if (response.import) onStarted(response.import.id);
      await queryClient.invalidateQueries({ queryKey: emojiKeys.adminImports });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });

  return (
    <section
      aria-label={t("admin.importTitle")}
      className="rounded-2xl border-2 border-line bg-surface p-4"
    >
      <h2 className="flex items-center gap-2 font-black">
        <Upload className="size-5 text-brand" />
        {t("admin.importTitle")}
      </h2>
      <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("admin.importLead")}</p>
      <button
        type="button"
        onClick={() => input.current?.click()}
        disabled={start.isPending}
        className={cn(
          "pressable mt-3 flex w-full items-center gap-3 rounded-2xl border-2 border-dashed border-ink-faint bg-surface-2 px-4 py-4 text-left",
          file && "border-solid border-ink bg-surface",
        )}
      >
        <FileArchive className="size-7 shrink-0 text-ink-soft" />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-bold">
            {file ? file.name : t("admin.importChoose")}
          </span>
          <span className="block text-xs text-ink-faint">
            {file ? formatBytes(BigInt(file.size)) : t("admin.importChooseHint")}
          </span>
        </span>
      </button>
      <input
        ref={input}
        type="file"
        accept=".zip,application/zip,application/x-zip-compressed"
        className="sr-only"
        tabIndex={-1}
        aria-label={t("admin.importChoose")}
        data-testid="import-file"
        onChange={(event) => {
          setFile(event.target.files?.[0]);
          event.target.value = "";
        }}
      />
      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        <SelectField<"skip" | "rename">
          label={t("admin.conflict")}
          value={policy}
          onChange={setPolicy}
          options={[
            { value: "skip", label: t("admin.conflictSkip"), hint: t("admin.conflictSkipHint") },
            {
              value: "rename",
              label: t("admin.conflictRename"),
              hint: t("admin.conflictRenameHint"),
            },
          ]}
        />
        <TextField
          label={t("admin.importCategory")}
          value={category}
          maxLength={32}
          onChange={(event) => setCategory(event.target.value)}
          placeholder={t("admin.importCategoryPlaceholder")}
        />
      </div>
      <Button
        className="mt-4"
        block
        loading={start.isPending}
        disabled={!file}
        onClick={() => file && start.mutate(file)}
      >
        {start.isPending
          ? t("form.uploadingPercent", { percent: Math.round(progress * 100) })
          : t("admin.importStart")}
      </Button>
    </section>
  );
}

const IMPORT_STATUS: Record<emojiApi.ImportStatus, string> = {
  [emojiApi.ImportStatus.UNSPECIFIED]: "queued",
  [emojiApi.ImportStatus.QUEUED]: "queued",
  [emojiApi.ImportStatus.RUNNING]: "running",
  [emojiApi.ImportStatus.COMPLETED]: "completed",
  [emojiApi.ImportStatus.FAILED]: "failed",
};
const ENTRY_STATUS: Record<emojiApi.ImportEntryStatus, { key: string; tone: string }> = {
  [emojiApi.ImportEntryStatus.UNSPECIFIED]: { key: "processing", tone: "" },
  [emojiApi.ImportEntryStatus.PROCESSING]: { key: "processing", tone: "" },
  [emojiApi.ImportEntryStatus.IMPORTED]: { key: "imported", tone: "bg-lime text-[#17122b]" },
  [emojiApi.ImportEntryStatus.RENAMED]: { key: "renamed", tone: "bg-sky text-[#17122b]" },
  [emojiApi.ImportEntryStatus.SKIPPED]: { key: "skipped", tone: "bg-sun text-[#17122b]" },
  [emojiApi.ImportEntryStatus.FAILED]: { key: "failed", tone: "bg-danger text-white" },
};

/** Entries of a pack read per request. */
const ENTRY_PAGE = 100;

const importRunning = (status: emojiApi.ImportStatus | undefined) =>
  status === emojiApi.ImportStatus.QUEUED || status === emojiApi.ImportStatus.RUNNING;

/** One import: how far it is, and what became of every entry of the pack. */
function ImportReport({ importId }: { importId: string }) {
  const { t } = useTranslation("emoji");
  // The first page carries the counters: it alone is read again on every progress event and poll.
  const head = useQuery({
    queryKey: emojiKeys.adminImport(importId),
    queryFn: () => emojiAdminClient().getImport({ importId, page: { limit: ENTRY_PAGE } }),
    // Progress arrives over the realtime connection (see `background.tsx`); this only covers a connection that dropped meanwhile.
    refetchInterval: (query) => (importRunning(query.state.data?.import?.status) ? 4000 : false),
  });
  const firstCursor = head.data?.page?.nextCursor ?? "";
  const running = importRunning(head.data?.import?.status);
  // The rest of a long pack is paged in once every entry has its outcome, and is not read again after that.
  const rest = useInfiniteQuery({
    queryKey: emojiKeys.adminImportRest(importId, firstCursor),
    queryFn: ({ pageParam }) =>
      emojiAdminClient().getImport({ importId, page: { limit: ENTRY_PAGE, cursor: pageParam } }),
    initialPageParam: firstCursor,
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: Boolean(firstCursor) && !running,
    staleTime: Number.POSITIVE_INFINITY,
  });
  if (!head.data) return head.isPending ? <Skeleton className="h-40" /> : null;
  const pack = head.data.import;
  if (!pack) return null;
  const entries = [
    ...head.data.entries,
    ...(rest.data?.pages.flatMap((page) => page.entries) ?? []),
  ];
  const settled = pack.importedCount + pack.skippedCount + pack.failedCount;
  const statusKey = IMPORT_STATUS[pack.status];

  return (
    <section
      aria-label={t("admin.reportTitle")}
      data-testid="import-report"
      data-status={statusKey}
      className="rounded-2xl border-2 border-line bg-surface p-4"
    >
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="mr-auto font-black">{t("admin.reportTitle")}</h2>
        <Chip
          className={cn(
            pack.status === emojiApi.ImportStatus.COMPLETED && "bg-lime text-[#17122b]",
            pack.status === emojiApi.ImportStatus.FAILED && "bg-danger text-white",
          )}
        >
          {t(`admin.importStatus.${statusKey}`)}
        </Chip>
      </div>
      {pack.sourceHost && (
        <p className="mt-0.5 text-xs text-ink-faint">
          {t("admin.importFrom", { host: pack.sourceHost })}
        </p>
      )}
      <div
        role="progressbar"
        aria-label={t("admin.reportProgress")}
        aria-valuemin={0}
        aria-valuemax={Math.max(1, pack.totalCount)}
        aria-valuenow={settled}
        className="mt-3 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
      >
        <div
          className={cn(
            "h-full bg-lime transition-[width] duration-500",
            running && pack.totalCount === 0 && "w-1/3 animate-pulse",
          )}
          style={
            pack.totalCount > 0 ? { width: `${(settled / pack.totalCount) * 100}%` } : undefined
          }
        />
      </div>
      <p className="mt-2 text-sm font-bold" data-testid="import-summary">
        {pack.status === emojiApi.ImportStatus.FAILED
          ? t(`admin.importErrors.${pack.error}`, {
              defaultValue: t("admin.importErrors.INTERNAL"),
            })
          : t("admin.reportSummary", {
              total: pack.totalCount,
              imported: pack.importedCount,
              skipped: pack.skippedCount,
              failed: pack.failedCount,
            })}
      </p>
      {entries.length > 0 && (
        <ul className="mt-3 max-h-80 divide-y-2 divide-line overflow-y-auto rounded-xl border-2 border-line">
          {entries.map((entry) => {
            const status = ENTRY_STATUS[entry.status];
            return (
              <li
                key={`${entry.sourceName}/${entry.fileName}`}
                data-testid="import-entry"
                data-status={status.key}
                className="flex flex-wrap items-center gap-x-2 gap-y-0.5 px-3 py-2 text-sm"
              >
                <span className="min-w-0 flex-1 truncate font-mono">
                  {entry.sourceName || entry.fileName}
                  {entry.name && entry.name !== entry.sourceName && (
                    <span className="text-ink-faint"> → :{entry.name}:</span>
                  )}
                </span>
                <Chip className={status.tone}>{t(`admin.entryStatus.${status.key}`)}</Chip>
                {entry.reason && (
                  <span className="w-full text-xs text-ink-soft">
                    {t(`admin.entryReasons.${entry.reason}`, { defaultValue: entry.reason })}
                  </span>
                )}
              </li>
            );
          })}
          {firstCursor && running ? (
            <li className="px-3 py-2 text-xs text-ink-faint">
              {t("admin.reportMore", { count: pack.totalCount - entries.length })}
            </li>
          ) : (
            <LoadMore query={rest} />
          )}
        </ul>
      )}
    </section>
  );
}

function ImportHistory({
  current,
  onOpen,
}: {
  current: string | undefined;
  onOpen(importId: string): void;
}) {
  const { t } = useTranslation("emoji");
  const dateTime = useDateTime();
  const list = useInfiniteQuery({
    queryKey: emojiKeys.adminImports,
    queryFn: ({ pageParam }) =>
      emojiAdminClient().listImports({ page: { limit: 10, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const imports = list.data?.pages.flatMap((page) => page.imports) ?? [];
  if (imports.length === 0) return null;
  return (
    <section
      aria-label={t("admin.importHistory")}
      className="rounded-2xl border-2 border-line bg-surface p-2"
    >
      <h2 className="px-2 pt-1.5 pb-1 text-sm font-black">{t("admin.importHistory")}</h2>
      <ul>
        {imports.map((pack) => (
          <li key={pack.id}>
            <button
              type="button"
              onClick={() => onOpen(pack.id)}
              aria-current={pack.id === current || undefined}
              className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-left text-sm hover:bg-surface-2 aria-[current]:bg-brand-soft"
            >
              <span className="min-w-0 flex-1 truncate">
                <span className="font-bold">{dateTime(pack.createdAt)}</span>
                <span className="text-ink-soft">
                  {" "}
                  ·{" "}
                  {t("admin.importCounts", {
                    imported: pack.importedCount,
                    total: pack.totalCount,
                  })}
                </span>
                {pack.requestedBy && (
                  <span className="text-ink-faint"> · @{pack.requestedBy.handle}</span>
                )}
              </span>
              <Chip>{t(`admin.importStatus.${IMPORT_STATUS[pack.status]}`)}</Chip>
            </button>
          </li>
        ))}
      </ul>
      <LoadMore query={list} />
    </section>
  );
}

// ───────────────────────────── Export ─────────────────────────────

const EXPORT_STATUS: Record<emojiApi.ExportStatus, string> = {
  [emojiApi.ExportStatus.UNSPECIFIED]: "queued",
  [emojiApi.ExportStatus.QUEUED]: "queued",
  [emojiApi.ExportStatus.RUNNING]: "running",
  [emojiApi.ExportStatus.READY]: "ready",
  [emojiApi.ExportStatus.FAILED]: "failed",
  [emojiApi.ExportStatus.EXPIRED]: "expired",
};

function Exports() {
  const { t } = useTranslation("emoji");
  const queryClient = useQueryClient();
  const dateTime = useDateTime();
  const categories = useCategories();
  const [scope, setScope] = useState(WHOLE);
  const list = useInfiniteQuery({
    queryKey: emojiKeys.adminExports,
    queryFn: ({ pageParam }) =>
      emojiAdminClient().listExports({ page: { limit: 10, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    // The realtime connection announces a finished export; this only covers a connection that dropped meanwhile.
    refetchInterval: (query) =>
      query.state.data?.pages[0]?.exports.some(
        (item) =>
          item.status === emojiApi.ExportStatus.QUEUED ||
          item.status === emojiApi.ExportStatus.RUNNING,
      )
        ? 4000
        : false,
  });
  const start = useMutation({
    mutationFn: () =>
      emojiAdminClient().startExport({
        category: scope === WHOLE ? undefined : scope === NONE ? "" : scope,
      }),
    onSuccess: async () => {
      toast.success(t("admin.exportStarted"));
      await queryClient.invalidateQueries({ queryKey: emojiKeys.adminExports });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  // A download link lasts an hour from the answer that carried it: ask for a fresh one at the click.
  const download = useMutation({
    mutationFn: (exportId: string) => emojiAdminClient().getExport({ exportId }),
    onSuccess: ({ export: file }) => {
      if (!file?.downloadUrl) return void toast.error(t("admin.exportGone"));
      const link = document.createElement("a");
      link.href = file.downloadUrl;
      link.download = "";
      link.rel = "noopener";
      document.body.append(link);
      link.click();
      link.remove();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  const exports = list.data?.pages.flatMap((page) => page.exports) ?? [];

  return (
    <section
      aria-label={t("admin.exportTitle")}
      className="rounded-2xl border-2 border-line bg-surface p-4"
    >
      <h2 className="flex items-center gap-2 font-black">
        <Download className="size-5 text-brand" />
        {t("admin.exportTitle")}
      </h2>
      <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("admin.exportLead")}</p>
      <div className="mt-3 flex flex-wrap items-end gap-2">
        <SelectField
          className="min-w-44 flex-1"
          label={t("admin.exportScope")}
          value={scope}
          onChange={setScope}
          options={[
            { value: WHOLE, label: t("admin.exportWhole") },
            ...(categories.data?.categories ?? []).map((entry) => ({
              value: entry.name,
              label: entry.name,
            })),
            { value: NONE, label: t("browse.uncategorized") },
          ]}
        />
        <Button loading={start.isPending} onClick={() => start.mutate()}>
          {t("admin.exportStart")}
        </Button>
      </div>
      {list.isPending ? (
        <Skeleton className="mt-4 h-20" />
      ) : exports.length === 0 ? (
        <p className="mt-4 rounded-xl bg-surface-2 px-4 py-5 text-center text-sm text-ink-soft">
          {t("admin.exportsEmpty")}
        </p>
      ) : (
        <ul className="mt-4 divide-y-2 divide-line rounded-xl border-2 border-line">
          {exports.map((file) => {
            const key = EXPORT_STATUS[file.status];
            return (
              <li
                key={file.id}
                data-testid="export-row"
                data-status={key}
                className="flex flex-wrap items-center gap-2 px-3 py-2 text-sm"
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-bold">
                    {file.byCategory
                      ? file.category || t("browse.uncategorized")
                      : t("admin.exportWhole")}
                  </span>
                  <span className="block truncate text-xs text-ink-soft">
                    {dateTime(file.createdAt)}
                    {file.status === emojiApi.ExportStatus.READY &&
                      ` · ${t("admin.exportMeta", { count: file.emojiCount, size: formatBytes(file.sizeBytes) })}`}
                    {file.requestedBy && ` · @${file.requestedBy.handle}`}
                  </span>
                </span>
                {file.status === emojiApi.ExportStatus.READY ? (
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={<Download className="size-4" />}
                    loading={download.isPending && download.variables === file.id}
                    onClick={() => download.mutate(file.id)}
                  >
                    {t("admin.download")}
                  </Button>
                ) : (
                  <Chip
                    className={cn(
                      file.status === emojiApi.ExportStatus.FAILED && "bg-danger text-white",
                    )}
                  >
                    {t(`admin.exportStatus.${key}`)}
                  </Chip>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <LoadMore query={list} />
    </section>
  );
}
