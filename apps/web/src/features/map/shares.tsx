import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type geo, toMillis } from "@tokuly/api";
import { Copy, Eye, Link2, Send, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { ShareToChat } from "#/features/chat";
import { copyText } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactDuration } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { locationClient, mapKeys, useLiveShares, useNow } from "./data";
import { useSharingState } from "./own-location";
import { MapSheet, SheetEmpty, SheetSection } from "./sheet";
import { useMapUi } from "./ui-store";

const DURATIONS = [
  { seconds: 15 * 60, key: "m15" },
  { seconds: 60 * 60, key: "h1" },
  { seconds: 4 * 60 * 60, key: "h4" },
  { seconds: 24 * 60 * 60, key: "h24" },
] as const;

/** The link as it works from where the app is open (the server names its own public address). */
export function shareLink(share: geo.LiveShare): string {
  try {
    return `${window.location.origin}${new URL(share.url).pathname}`;
  } catch {
    return share.url;
  }
}

/** Live shares: my position for someone outside Tokuly, by link, for a set time. */
export function SharesPanel() {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const { stack, back, close } = useMapUi();
  const { data: shares, isPending } = useLiveShares();
  const sharing = useSharingState();
  const now = useNow(30_000);
  const [duration, setDuration] = useState<number>(DURATIONS[1].seconds);
  const [label, setLabel] = useState("");
  const [revoking, setRevoking] = useState<geo.LiveShare | null>(null);
  const [sending, setSending] = useState<geo.LiveShare | null>(null);

  const create = useMutation({
    mutationFn: () =>
      locationClient().createLiveShare({ durationSeconds: duration, label: label.trim() }),
    onSuccess: async (response) => {
      setLabel("");
      await queryClient.invalidateQueries({ queryKey: mapKeys.shares });
      if (response.share && (await copyText(shareLink(response.share))))
        toast.success(t("shares.createdCopied"));
      else toast.success(t("shares.created"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });
  const revoke = useMutation({
    mutationFn: (share: geo.LiveShare) => locationClient().revokeLiveShare({ shareId: share.id }),
    onSuccess: (_response, share) => {
      queryClient.setQueryData<geo.LiveShare[]>(mapKeys.shares, (list) =>
        (list ?? []).filter((entry) => entry.id !== share.id),
      );
      setRevoking(null);
      toast.success(t("shares.revoked"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "map" })),
  });

  const active = (shares ?? []).filter((share) => toMillis(share.expiresAt) > now);
  return (
    <MapSheet
      testId="shares-panel"
      title={t("shares.title")}
      subtitle={t("shares.subtitle")}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      <SheetSection title={t("shares.new")}>
        <div className="rounded-2xl bg-surface-2 p-3">
          <div
            className="grid grid-cols-4 gap-1.5"
            role="radiogroup"
            aria-label={t("shares.duration")}
          >
            {DURATIONS.map((option) => (
              // biome-ignore lint/a11y/useSemanticElements: tappable chips, not radio inputs.
              <button
                key={option.key}
                type="button"
                role="radio"
                aria-checked={duration === option.seconds}
                onClick={() => setDuration(option.seconds)}
                className={cn(
                  "pressable h-10 rounded-full text-xs font-black",
                  duration === option.seconds ? "sticker-sm bg-brand text-on-brand" : "bg-surface",
                )}
              >
                {t(`shares.durations.${option.key}`)}
              </button>
            ))}
          </div>
          <TextField
            className="mt-3"
            label={t("shares.label")}
            placeholder={t("shares.labelPlaceholder")}
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            maxLength={60}
          />
          <Button
            className="mt-3"
            block
            icon={<Link2 className="size-4" />}
            loading={create.isPending}
            onClick={() => create.mutate()}
          >
            {t("shares.create")}
          </Button>
          {sharing !== "sharing" && sharing !== "starting" && (
            <p className="mt-2 text-[11px] leading-relaxed font-bold text-danger">
              {t("shares.notSharing")}
            </p>
          )}
        </div>
      </SheetSection>

      <SheetSection title={t("shares.active")}>
        {isPending ? (
          <Skeleton className="h-16" />
        ) : active.length === 0 ? (
          <SheetEmpty emoji="🔗" title={t("shares.emptyTitle")} body={t("shares.emptyBody")} />
        ) : (
          <ul className="space-y-2">
            {active.map((share) => (
              <li key={share.id} data-testid="share-row" className="rounded-2xl bg-surface-2 p-3">
                <p className="truncate text-sm font-black">{share.label || t("shares.untitled")}</p>
                <p className="mt-0.5 flex items-center gap-2 text-[11px] font-bold text-ink-soft">
                  <span>
                    {t("shares.endsIn", { time: compactDuration(toMillis(share.expiresAt) - now) })}
                  </span>
                  <span className="flex items-center gap-0.5">
                    <Eye className="size-3" /> {t("shares.views", { count: share.viewCount })}
                  </span>
                </p>
                <p
                  className="mt-1.5 truncate rounded-lg bg-surface px-2 py-1 font-mono text-[11px] text-ink-soft"
                  data-testid="share-url"
                >
                  {shareLink(share)}
                </p>
                <div className="mt-2 flex gap-1.5">
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={<Copy className="size-3.5" />}
                    onClick={async () =>
                      toast.success(
                        (await copyText(shareLink(share))) ? t("shares.copied") : shareLink(share),
                      )
                    }
                  >
                    {t("shares.copy")}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={<Send className="size-3.5" />}
                    onClick={() => setSending(share)}
                  >
                    {t("shares.sendInChat")}
                  </Button>
                  <span className="flex-1" />
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("shares.revoke")}
                    title={t("shares.revoke")}
                    onClick={() => setRevoking(share)}
                  >
                    <Trash2 className="size-4 text-danger" />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </SheetSection>

      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(open) => !open && setRevoking(null)}
        title={t("shares.revokeTitle")}
        description={t("shares.revokeBody")}
        confirmLabel={t("shares.revoke")}
        danger
        loading={revoke.isPending}
        onConfirm={() => revoking && revoke.mutate(revoking)}
      />
      {sending && (
        <ShareToChat
          open
          onOpenChange={(open) => !open && setSending(null)}
          share={{ url: shareLink(sending) }}
          title={t("shares.chatTitle")}
        />
      )}
    </MapSheet>
  );
}
