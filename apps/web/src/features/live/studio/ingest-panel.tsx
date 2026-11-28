import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason } from "@tokuly/api";
import { KeyRound, RefreshCw } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { streamClient } from "../api";
import { CopyField, SecretField } from "./shared";

/** Where the ingest details live in the cache. They are secrets: kept in memory only while the panel is open. */
export const ingestKey = (streamId: string) => ["live", "ingest", streamId] as const;

/** The stream's ingest details (the key and the URLs that carry it), for its owner. */
export function useIngestInfo(streamId: string, enabled = true) {
  return useQuery({
    queryKey: ingestKey(streamId),
    queryFn: async () => (await streamClient().getIngestInfo({ streamId })).ingest ?? null,
    enabled,
    staleTime: 60_000,
    // Dropped as soon as nothing shows them.
    gcTime: 0,
    retry: (count, error) => errorReason(error) === "" && count < 2,
  });
}

/**
 * What OBS (or any encoder) needs to publish this stream: the RTMP server and
 * key, and the SRT and WHIP addresses. Everything that contains the key is
 * masked until the owner asks to see it; the key can be replaced, which cuts
 * off whatever publishes with the old one.
 */
export function IngestPanel({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const query = useIngestInfo(streamId);
  const [rotating, setRotating] = useState(false);
  const rotate = useMutation({
    mutationFn: () => streamClient().rotateStreamKey({ streamId }),
    onSuccess: ({ ingest }) => {
      queryClient.setQueryData(ingestKey(streamId), ingest ?? null);
      setRotating(false);
      toast.success(t("ingest.rotated"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  if (query.isPending)
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-12 w-full" />
      </div>
    );
  const ingest = query.data;
  if (query.isError || !ingest)
    return (
      <p
        className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft"
        role="alert"
      >
        {query.error ? errorMessage(query.error, { namespace: "live" }) : t("ingest.unavailable")}
      </p>
    );

  return (
    <div className="flex flex-col gap-4" data-testid="ingest-panel">
      <p className="text-sm leading-relaxed text-ink-soft">{t("ingest.lead")}</p>
      <div className="flex flex-col gap-3">
        <h3 className="text-sm font-black">{t("ingest.rtmp")}</h3>
        <CopyField label={t("ingest.rtmpServer")} value={ingest.rtmpServerUrl} />
        <SecretField
          label={t("ingest.streamKey")}
          value={ingest.rtmpStreamKey}
          hint={t("ingest.streamKeyHint")}
        />
      </div>
      <div className="flex flex-col gap-3">
        <h3 className="text-sm font-black">{t("ingest.other")}</h3>
        <SecretField label={t("ingest.srt")} value={ingest.srtUrl} kind="url" />
        <CopyField label={t("ingest.whip")} value={ingest.whipUrl} />
        <SecretField label={t("ingest.whipToken")} value={ingest.whipBearerToken} />
      </div>
      <div className="flex flex-wrap items-center gap-3 rounded-2xl bg-surface-2 px-4 py-3">
        <KeyRound className="size-5 shrink-0 text-ink-soft" />
        <p className="min-w-0 flex-1 text-sm text-ink-soft">{t("ingest.rotateLead")}</p>
        <Button
          variant="secondary"
          size="sm"
          icon={<RefreshCw className="size-4" />}
          onClick={() => setRotating(true)}
        >
          {t("ingest.rotate")}
        </Button>
      </div>
      <ConfirmDialog
        open={rotating}
        onOpenChange={setRotating}
        title={t("ingest.rotateTitle")}
        description={t("ingest.rotateBody")}
        confirmLabel={t("ingest.rotate")}
        danger
        loading={rotate.isPending}
        onConfirm={() => rotate.mutate()}
      />
    </div>
  );
}
