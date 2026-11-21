import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { stamp } from "@tokuly/api";
import { ChevronRight, Download, Heart, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { libraryClient, patchStamp, refreshHoldings, stampKeys, storeClient, useToggleFavorite } from "./api";
import { PackCover, PackMaker, PriceTag } from "./parts";
import { PurchaseDialog } from "./purchase";
import { StampImage } from "./stamp-image";
import { parseKeywords, type StampMeta, StampMetaFields } from "./stamp-meta";

export interface StampSheetProps {
  /** The stamp to show. The sheet is open while this is set. */
  stampId: string | undefined;
  /** Called when the person closes the sheet: clear `stampId`. */
  onClose(): void;
}

/**
 * What to show when someone taps a stamp — one they received in a chat, or
 * one in a grid: the stamp at full size (animations play), a favourite
 * button, "save" for stamps they do not have yet (adds its pack, or copies a
 * friend's personal stamp), the pack it comes from, and rename / delete for
 * their own personal stamps.
 */
export function StampSheet({ stampId, onClose }: StampSheetProps) {
  // The last stamp stays rendered while the sheet animates out.
  const [shown, setShown] = useState(stampId);
  if (stampId && stampId !== shown) setShown(stampId);
  return shown ? <Sheet key={shown} stampId={shown} open={Boolean(stampId)} onClose={onClose} /> : null;
}

function Sheet({ stampId, open, onClose }: { stampId: string; open: boolean; onClose(): void }) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const { data, isPending, isError } = useQuery({
    queryKey: stampKeys.stamp(stampId),
    queryFn: () => storeClient().getStamp({ stampId }),
    enabled: open,
    staleTime: 0,
  });
  const item = data?.stamp;
  const ref = item?.stampRef;
  const pack = data?.pack;
  const mine = Boolean(item?.personal && item.usable);

  const favorite = useToggleFavorite();
  const [editing, setEditing] = useState<StampMeta | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [buying, setBuying] = useState<stamp.PackSummary | null>(null);

  const refresh = () => Promise.all([refreshHoldings(queryClient), queryClient.invalidateQueries({ queryKey: stampKeys.stamp(stampId) })]);
  const onError = (error: unknown) => toast.error(errorMessage(error));

  const save = useMutation({
    mutationFn: () => libraryClient().saveStamp({ stampId }),
    onSuccess: (response) => {
      switch (response.outcome) {
        case stamp.SaveOutcome.PACK_INSTALLED:
          toast.success(t("toast.savedPack", { name: response.pack?.name ?? "" }));
          break;
        case stamp.SaveOutcome.SAVED_AS_PERSONAL:
          toast.success(t("toast.savedPersonal"));
          break;
        case stamp.SaveOutcome.PURCHASE_REQUIRED:
          if (response.pack) setBuying(response.pack);
          break;
        default:
          toast(t("toast.alreadySaved"));
      }
      return refresh();
    },
    onError,
  });
  const rename = useMutation({
    mutationFn: (meta: StampMeta) =>
      libraryClient().updatePersonalStamp({ stampId, name: meta.name.trim(), emoji: meta.emoji, keywords: { keywords: parseKeywords(meta.keywords) } }),
    onSuccess: (response) => {
      if (response.stamp) patchStamp(queryClient, stampId, response.stamp);
      setEditing(null);
      toast.success(t("toast.renamed"));
      return refresh();
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => libraryClient().deletePersonalStamp({ stampId }),
    onSuccess: () => {
      setConfirmDelete(false);
      onClose();
      toast.success(t("toast.deleted"));
      return refreshHoldings(queryClient);
    },
    onError,
  });

  const title = editing ? t("stamp.edit") : ref?.name || (isPending ? " " : t("stamp.unnamed"));

  return (
    <>
      <Dialog open={open} onOpenChange={(next) => !next && onClose()} title={title}>
        {isPending ? (
          <div className="flex flex-col items-center gap-4">
            <Skeleton className="size-48 rounded-[2rem]" />
            <Skeleton className="h-11 w-56 rounded-full" />
          </div>
        ) : isError || !item || !ref ? (
          <EmptyState emoji="🫥" title={t("stamp.gone")} body={t("stamp.goneBody")} />
        ) : editing ? (
          <form
            className="flex flex-col gap-4"
            onSubmit={(event) => {
              event.preventDefault();
              rename.mutate(editing);
            }}
          >
            <StampImage stamp={ref} image={item.image} size={96} play="always" className="self-center" />
            <StampMetaFields value={editing} onChange={setEditing} autoFocus />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setEditing(null)} disabled={rename.isPending}>
                {t("common:actions.cancel")}
              </Button>
              <Button type="submit" loading={rename.isPending}>
                {t("common:actions.save")}
              </Button>
            </div>
          </form>
        ) : (
          <div className="flex flex-col items-center gap-4">
            <div className="grid w-full place-items-center rounded-[2rem] bg-surface-2 py-6">
              <StampImage stamp={ref} size={208} play="always" className="animate-pop-in" />
            </div>

            {(item.emoji || item.keywords.length > 0) && (
              <div className="flex flex-wrap justify-center gap-1.5">
                {item.emoji && <Chip className="text-sm">{item.emoji}</Chip>}
                {item.keywords.map((keyword) => (
                  <Chip key={keyword}>{keyword}</Chip>
                ))}
              </div>
            )}

            <div className="flex flex-wrap justify-center gap-2">
              {item.usable && (
                <Button
                  variant="secondary"
                  aria-pressed={item.favorited}
                  icon={<Heart className={cn("size-4", item.favorited && "fill-pink text-pink")} />}
                  onClick={() => favorite.mutate({ stampId, favorited: !item.favorited })}
                >
                  {item.favorited ? t("stamp.unfavorite") : t("stamp.favorite")}
                </Button>
              )}
              {data.canSave && (
                <Button variant="lime" icon={<Download className="size-4" />} loading={save.isPending} onClick={() => save.mutate()}>
                  {t("stamp.save")}
                </Button>
              )}
              {mine && (
                <>
                  <Button
                    variant="secondary"
                    icon={<Pencil className="size-4" />}
                    onClick={() => setEditing({ name: ref.name, emoji: item.emoji, keywords: item.keywords.join(" ") })}
                  >
                    {t("stamp.edit")}
                  </Button>
                  <Button variant="ghost" className="text-danger" icon={<Trash2 className="size-4" />} onClick={() => setConfirmDelete(true)}>
                    {t("common:actions.delete")}
                  </Button>
                </>
              )}
            </div>

            {item.personal && !mine && <p className="text-center text-sm text-ink-soft">{t("stamp.personalOfOther")}</p>}
            {mine && <p className="text-center text-sm text-ink-soft">{t("stamp.personalMine")}</p>}

            {pack && (
              <Link
                to="/stamps/$packId"
                params={{ packId: pack.shortId || pack.id }}
                onClick={onClose}
                className="pressable sticker flex w-full items-center gap-3 rounded-card bg-surface p-3"
              >
                <PackCover pack={pack} className="size-14 shrink-0 rounded-xl" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">{pack.name}</span>
                  <span className="flex min-w-0 items-center gap-2 text-xs">
                    <PackMaker pack={pack} />
                    <PriceTag pack={pack} className="shrink-0" />
                  </span>
                </span>
                <ChevronRight className="size-5 shrink-0 text-ink-faint" />
              </Link>
            )}
          </div>
        )}
      </Dialog>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("stamp.deleteTitle")}
        description={t("stamp.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      {buying && <PurchaseDialog pack={buying} open onOpenChange={(next) => !next && setBuying(null)} />}
    </>
  );
}
