import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { media, stamp, toDate } from "@tokuly/api";
import {
  Crown,
  ExternalLink,
  FilePlus2,
  MoreHorizontal,
  Pencil,
  Scissors,
  Send,
  Trash2,
  Undo2,
} from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { stampKeys, studioClient, usePack } from "./api";
import { StampMaker } from "./editor/stamp-maker";
import { netEarnings, PackFormDialog, type PackListing } from "./pack-form";
import { PackStats } from "./pack-stats";
import { PackCover, SectionHeading, StampTile, StatusChip } from "./parts";
import { StampImage } from "./stamp-image";
import { nameFromFile, parseKeywords, type StampMeta, StampMetaFields } from "./stamp-meta";

/** Files the "add files" button takes: still and animated images, and dotLottie animations. */
const READY_MADE = "image/png,image/webp,image/gif,image/apng,image/jpeg,.lottie";
/** Stamps one AddStamps call may carry. */
const ADD_BATCH = 30;

/** One pack in the creator studio: its listing, its stamps, its way through review, and its numbers. */
export function StudioPackScreen({ packId }: { packId: string }) {
  const { t } = useTranslation("stamps");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: pack, isPending, isError } = usePack(packId);
  const profile = useQuery({
    queryKey: stampKeys.creatorProfile,
    queryFn: async () => (await studioClient().getCreatorProfile({})).profile,
  });

  const [editingListing, setEditingListing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmUnpublish, setConfirmUnpublish] = useState(false);
  const [making, setMaking] = useState(false);
  const [editingStamp, setEditingStamp] = useState<string>();
  const [upload, setUpload] = useState<{ done: number; total: number } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const summary = pack?.summary;
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "stamps" }));

  /** Shows the pack as the server now has it, and lets every list that includes it catch up. */
  const apply = (next: stamp.Pack | undefined) => {
    if (next?.summary) {
      for (const key of new Set([packId, next.summary.id, next.summary.shortId]))
        queryClient.setQueryData(stampKeys.pack(key), next);
    }
    for (const queryKey of [stampKeys.studio, stampKeys.library, stampKeys.store, stampKeys.review])
      void queryClient.invalidateQueries({ queryKey });
  };

  const submit = useMutation({
    mutationFn: () => studioClient().submitPack({ packId: summary!.id }),
    onSuccess: (response) => {
      apply(response.pack);
      toast.success(
        response.pack?.summary?.status === stamp.PackStatus.PUBLISHED
          ? t("toast.published")
          : t("toast.submitted"),
      );
    },
    onError,
  });
  const unpublish = useMutation({
    mutationFn: () => studioClient().unpublishPack({ packId: summary!.id }),
    onSuccess: (response) => {
      apply(response.pack);
      setConfirmUnpublish(false);
      toast.success(t("toast.unpublished"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => studioClient().deletePack({ packId: summary!.id }),
    onSuccess: async () => {
      setConfirmDelete(false);
      toast.success(t("toast.packDeleted"));
      await navigate({ to: "/stamps/studio" });
      await queryClient.invalidateQueries({ queryKey: stampKeys.all });
    },
    onError,
  });
  const reorder = useMutation({
    mutationFn: (ordered: stamp.Stamp[]) =>
      studioClient().reorderStamps({
        packId: summary!.id,
        stampIds: ordered.map((item) => item.stampRef!.id),
      }),
    // The grid keeps the dropped order while the server confirms it.
    onMutate: (ordered) =>
      queryClient.setQueryData<stamp.Pack>(
        stampKeys.pack(packId),
        (current) => current && { ...current, stamps: ordered },
      ),
    onSuccess: (response) => apply(response.pack),
    onError: (error) => {
      onError(error);
      return queryClient.invalidateQueries({ queryKey: stampKeys.pack(packId) });
    },
  });

  const saveListing = async (listing: PackListing) => {
    try {
      const response = await studioClient().updatePack({
        packId: summary!.id,
        name: listing.name,
        description: listing.description,
        tags: { tags: listing.tags },
        price: listing.price,
      });
      apply(response.pack);
      toast.success(t("toast.packSaved"));
    } catch (error) {
      onError(error);
      throw error;
    }
  };

  /** Uploads ready-made files one after another and adds what made it to the pack. */
  const addFiles = async (files: File[]) => {
    if (!summary || files.length === 0) return;
    const room = (profile.data?.maxStampsPerPack ?? 120) - summary.stampCount;
    const accepted = files.slice(0, Math.max(0, room));
    if (accepted.length < files.length)
      toast(t("studio.pack.tooMany", { count: Math.max(0, room) }));
    if (accepted.length === 0) return;

    setUpload({ done: 0, total: accepted.length });
    const ready: { assetId: string; name: string }[] = [];
    let failed = 0;
    for (const [index, file] of accepted.entries()) {
      try {
        const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.STAMP });
        ready.push({ assetId: asset.id, name: nameFromFile(file.name) });
      } catch {
        failed++;
      }
      setUpload({ done: index + 1, total: accepted.length });
    }
    try {
      for (let start = 0; start < ready.length; start += ADD_BATCH) {
        apply(
          (
            await studioClient().addStamps({
              packId: summary.id,
              stamps: ready.slice(start, start + ADD_BATCH),
            })
          ).pack,
        );
      }
      if (ready.length > 0) toast.success(t("toast.stampsAdded", { count: ready.length }));
    } catch (error) {
      onError(error);
    }
    if (failed > 0) toast.error(t("studio.pack.uploadFailed", { count: failed }));
    setUpload(null);
  };

  if (isPending) {
    return (
      <Page title=" " back="/stamps/studio" wide>
        <div className="grid gap-5 md:grid-cols-[22rem_1fr]">
          <Skeleton className="h-80" />
          <Skeleton className="h-96" />
        </div>
      </Page>
    );
  }
  if (isError || !pack || !summary || !summary.viewer?.canManage) {
    return (
      <Page title={t("studio.title")} back="/stamps/studio">
        <EmptyState
          emoji="🗝️"
          title={t("studio.pack.notYours")}
          body={t("studio.pack.notYoursBody")}
        />
      </Page>
    );
  }

  const status = summary.status;
  const inReview = status === stamp.PackStatus.IN_REVIEW;
  const live = status === stamp.PackStatus.PUBLISHED;
  // A published pack takes new stamps only from creators who skip review (and for official packs).
  const canAdd = !inReview && (!live || summary.official || Boolean(profile.data?.trusted));
  const full = summary.stampCount >= (profile.data?.maxStampsPerPack ?? 120);
  const coverId = pack.coverStampId || pack.stamps[0]?.stampRef?.id;
  // In a pack that never went live every stamp awaits review; the mark only helps once some are approved.
  const allPending = pack.stamps.every((item) => item.pendingReview);
  const editing = pack.stamps.find((item) => item.stampRef?.id === editingStamp);

  return (
    <Page
      title={summary.name}
      back="/stamps/studio"
      wide
      actions={
        <Menu
          trigger={
            <Button size="icon-sm" variant="ghost" aria-label={t("studio.pack.menu")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            {
              label: t("studio.pack.viewInStore"),
              icon: <ExternalLink />,
              onSelect: () =>
                void navigate({
                  to: "/stamps/$packId",
                  params: { packId: summary.shortId || summary.id },
                }),
            },
            {
              label: t("studio.pack.delete"),
              icon: <Trash2 />,
              danger: true,
              onSelect: () => setConfirmDelete(true),
            },
          ]}
        />
      }
    >
      <div className="grid items-start gap-5 md:grid-cols-[22rem_1fr]">
        <div className="flex flex-col gap-4">
          <StatusPanel
            pack={pack}
            submitting={submit.isPending}
            withdrawing={unpublish.isPending}
            onSubmit={() => submit.mutate()}
            onWithdraw={() => (live ? setConfirmUnpublish(true) : unpublish.mutate())}
          />

          <section
            className="sticker flex flex-col gap-3 rounded-card bg-surface p-4"
            aria-label={t("studio.pack.listing")}
          >
            <div className="flex items-center gap-3">
              <PackCover pack={summary} className="size-20 shrink-0" />
              <div className="min-w-0 flex-1">
                <h2 className="text-lg leading-tight font-black break-words">{summary.name}</h2>
                <p className="mt-1 text-sm font-bold text-ink-soft">
                  {summary.price > 0 ? <Coins amount={summary.price} size="sm" /> : t("free")}
                  {summary.official && <span className="ml-2 text-brand">{t("official")}</span>}
                </p>
              </div>
            </div>
            {pack.description ? (
              <p className="text-sm leading-relaxed break-words whitespace-pre-wrap">
                {pack.description}
              </p>
            ) : (
              <p className="text-sm text-ink-faint">{t("studio.pack.noDescription")}</p>
            )}
            {summary.tags.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {summary.tags.map((tag) => (
                  <Chip key={tag}>#{tag}</Chip>
                ))}
              </div>
            )}
            {summary.price > 0 && !summary.official && profile.data && (
              <p className="flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
                {t("form.earnings", { fee: profile.data.feePercent })}
                <Coins
                  amount={netEarnings(summary.price, profile.data.feePercent)}
                  size="sm"
                  className="text-ink"
                />
              </p>
            )}
            <Button
              variant="secondary"
              size="sm"
              className="self-start"
              icon={<Pencil className="size-4" />}
              disabled={inReview}
              onClick={() => setEditingListing(true)}
            >
              {t("studio.pack.editListing")}
            </Button>
          </section>
        </div>

        <div className="flex min-w-0 flex-col gap-6">
          <section aria-label={t("pack.stamps")}>
            <SectionHeading
              title={
                <>
                  {t("pack.stamps")}
                  <span className="ml-2 text-sm font-bold text-ink-faint tabular-nums">
                    {summary.stampCount} / {profile.data?.maxStampsPerPack ?? 120}
                  </span>
                </>
              }
            />
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Button
                size="sm"
                variant="lime"
                icon={<Scissors className="size-4" />}
                disabled={!canAdd || full || upload !== null}
                onClick={() => setMaking(true)}
              >
                {t("studio.pack.makeStamp")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                icon={<FilePlus2 className="size-4" />}
                disabled={!canAdd || full}
                loading={upload !== null}
                onClick={() => fileInput.current?.click()}
              >
                {upload ? t("studio.pack.uploading", upload) : t("studio.pack.addFiles")}
              </Button>
              <input
                ref={fileInput}
                type="file"
                multiple
                accept={READY_MADE}
                className="sr-only"
                tabIndex={-1}
                aria-label={t("studio.pack.addFiles")}
                onChange={(event) => {
                  const files = [...(event.target.files ?? [])];
                  // Clearing the field lets the same files be chosen again.
                  event.target.value = "";
                  void addFiles(files);
                }}
              />
            </div>
            <p className="mb-2 px-1 text-xs leading-relaxed text-ink-faint">
              {inReview
                ? t("studio.pack.lockedInReview")
                : live && !canAdd
                  ? t("studio.pack.lockedLive")
                  : pack.stamps.length > 1
                    ? t("studio.pack.stampsHint")
                    : t("studio.pack.filesHint")}
            </p>

            {pack.stamps.length === 0 ? (
              <EmptyState
                emoji="🖼️"
                title={t("studio.pack.noStamps")}
                body={t("studio.pack.noStampsBody")}
              />
            ) : (
              <Sortable
                items={pack.stamps}
                getKey={(item) => item.stampRef?.id ?? ""}
                onReorder={(ordered) => reorder.mutate(ordered)}
                grab="press"
                disabled={inReview}
                className="sticker grid grid-cols-4 gap-1 rounded-card bg-surface p-2 sm:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8"
              >
                {(item, { grip, dragging }) => (
                  <div {...grip} className={cn("rounded-2xl", dragging && "bg-brand-soft")}>
                    <StampTile
                      item={item}
                      hidePending={allPending}
                      onPress={(pressed) => setEditingStamp(pressed.stampRef?.id)}
                    >
                      {item.stampRef?.id === coverId && (
                        <span className="pointer-events-none absolute top-0.5 left-0.5 grid size-5 place-items-center rounded-full border-2 border-ink bg-sun text-[#17122b]">
                          <Crown className="size-2.5" aria-label={t("studio.pack.cover")} />
                        </span>
                      )}
                    </StampTile>
                  </div>
                )}
              </Sortable>
            )}
          </section>

          <section aria-label={t("stats.title")}>
            <SectionHeading title={t("stats.title")} />
            {summary.publishedAt || summary.installCount > 0n ? (
              <PackStats packId={summary.id} paid={summary.price > 0} />
            ) : (
              <p className="rounded-2xl border-2 border-dashed border-line px-4 py-8 text-center text-sm font-bold text-ink-faint">
                {t("stats.notYet")}
              </p>
            )}
          </section>
        </div>
      </div>

      <PackFormDialog
        open={editingListing}
        onOpenChange={setEditingListing}
        title={t("studio.pack.editListing")}
        submitLabel={t("common:actions.save")}
        initial={{
          name: summary.name,
          description: pack.description,
          tags: summary.tags,
          price: summary.price,
        }}
        limits={profile.data}
        official={summary.official}
        onSubmit={saveListing}
      />
      <StampMaker
        open={making}
        onOpenChange={setMaking}
        title={t("studio.pack.makeTitle")}
        submitLabel={t("studio.pack.makeSubmit")}
        onSubmit={async (made) => {
          apply((await studioClient().addStamps({ packId: summary.id, stamps: [made] })).pack);
          toast.success(t("toast.stampsAdded", { count: 1 }));
        }}
      />
      {editing && (
        <StampEditor
          key={editingStamp}
          item={editing}
          locked={inReview}
          isCover={editing.stampRef?.id === coverId}
          allPending={allPending}
          packId={summary.id}
          onClose={() => setEditingStamp(undefined)}
          onChanged={apply}
        />
      )}
      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("studio.pack.deleteTitle")}
        description={t("studio.pack.deleteBody", { name: summary.name })}
        confirmLabel={t("studio.pack.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <ConfirmDialog
        open={confirmUnpublish}
        onOpenChange={setConfirmUnpublish}
        title={t("studio.pack.unpublishTitle")}
        description={t("studio.pack.unpublishBody")}
        confirmLabel={t("studio.pack.unpublish")}
        loading={unpublish.isPending}
        onConfirm={() => unpublish.mutate()}
      />
    </Page>
  );
}

const PANEL_TINT: Record<number, string> = {
  [stamp.PackStatus.DRAFT]: "bg-surface",
  [stamp.PackStatus.IN_REVIEW]: "bg-sun text-[#17122b]",
  [stamp.PackStatus.PUBLISHED]: "bg-lime text-[#17122b]",
  [stamp.PackStatus.REJECTED]: "bg-surface",
  [stamp.PackStatus.REMOVED]: "bg-surface",
};

/** Where the pack is on its way to the store, and the one step that moves it on. */
function StatusPanel({
  pack,
  submitting,
  withdrawing,
  onSubmit,
  onWithdraw,
}: {
  pack: stamp.Pack;
  submitting: boolean;
  withdrawing: boolean;
  onSubmit(): void;
  onWithdraw(): void;
}) {
  const { t, i18n } = useTranslation("stamps");
  const summary = pack.summary!;
  const review = pack.review;
  const status = summary.status;
  const declined = status === stamp.PackStatus.REJECTED || status === stamp.PackStatus.REMOVED;
  const submittable = status === stamp.PackStatus.DRAFT || declined;
  const submitted = toDate(review?.submittedAt);
  // On coloured panels the secondary text takes the panel's ink instead of the theme's soft ink.
  const coloured = status === stamp.PackStatus.IN_REVIEW || status === stamp.PackStatus.PUBLISHED;
  const soft = coloured ? "text-[#17122b]/75" : "text-ink-soft";

  const lead: Record<number, string> = {
    [stamp.PackStatus.DRAFT]: review?.needsReview
      ? t("studio.pack.draftLead")
      : t("studio.pack.draftLeadTrusted"),
    [stamp.PackStatus.IN_REVIEW]: t("studio.pack.inReviewLead"),
    [stamp.PackStatus.PUBLISHED]: t("studio.pack.publishedLead"),
    [stamp.PackStatus.REJECTED]: t("studio.pack.rejectedLead"),
    [stamp.PackStatus.REMOVED]: t("studio.pack.removedLead"),
  };

  let action: ReactNode = null;
  if (submittable) {
    action = (
      <Button
        icon={<Send className="size-4" />}
        loading={submitting}
        disabled={summary.stampCount === 0}
        onClick={onSubmit}
      >
        {declined
          ? t("studio.pack.resubmit")
          : review?.needsReview
            ? t("studio.pack.submit")
            : t("studio.pack.publish")}
      </Button>
    );
  } else {
    action = (
      <Button
        variant="secondary"
        icon={<Undo2 className="size-4" />}
        loading={withdrawing}
        onClick={onWithdraw}
      >
        {status === stamp.PackStatus.IN_REVIEW
          ? t("studio.pack.withdraw")
          : t("studio.pack.unpublish")}
      </Button>
    );
  }

  return (
    <section
      className={cn(
        "sticker flex flex-col gap-3 rounded-card p-4",
        PANEL_TINT[status] ?? "bg-surface",
      )}
      aria-label={t("studio.pack.status")}
      data-testid="pack-status"
    >
      <div className="flex items-center gap-2">
        <StatusChip
          status={status}
          className={coloured ? "border-2 border-ink bg-surface text-ink" : undefined}
        />
        {status === stamp.PackStatus.IN_REVIEW && submitted && (
          <span className={cn("text-xs font-bold", soft)}>
            {t("studio.pack.submittedOn", { date: submitted.toLocaleDateString(i18n.language) })}
          </span>
        )}
      </div>
      <p className={cn("text-sm leading-relaxed font-bold", soft)}>
        {lead[status] ?? lead[stamp.PackStatus.DRAFT]}
      </p>
      {declined && review?.note && (
        <blockquote
          className="rounded-2xl border-2 border-danger bg-surface px-3 py-2.5 text-sm leading-relaxed break-words whitespace-pre-wrap"
          data-testid="review-note"
        >
          <span className="block text-xs font-black text-danger">{t("studio.pack.reason")}</span>
          {review.note}
        </blockquote>
      )}
      {(review?.pendingStampCount ?? 0) > 0 &&
        (review?.pendingStampCount ?? 0) < summary.stampCount &&
        status !== stamp.PackStatus.IN_REVIEW && (
          <p className={cn("text-xs font-bold", soft)}>
            {t("studio.pack.pendingStamps", { count: review?.pendingStampCount ?? 0 })}
          </p>
        )}
      {submittable && summary.stampCount === 0 && (
        <p className="text-xs font-bold text-ink-faint">{t("studio.pack.needStamps")}</p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {action}
        {status === stamp.PackStatus.PUBLISHED && (
          <Link
            to="/stamps/$packId"
            params={{ packId: summary.shortId || summary.id }}
            className="inline-flex h-11 items-center gap-1.5 rounded-full px-3 text-sm font-bold underline-offset-4 hover:underline"
          >
            <ExternalLink className="size-4" />
            {t("studio.pack.viewInStore")}
          </Link>
        )}
      </div>
    </section>
  );
}

/** Edits one stamp of a pack: its name and search words, whether it is the cover, or removes it. */
function StampEditor({
  item,
  locked,
  isCover,
  allPending,
  packId,
  onClose,
  onChanged,
}: {
  item: stamp.Stamp;
  /** Every stamp of the pack awaits review, so saying it of this one tells nothing. */
  allPending: boolean;
  /** The pack is in review: nothing can be changed. */
  locked: boolean;
  isCover: boolean;
  packId: string;
  onClose(): void;
  onChanged(pack: stamp.Pack | undefined): void;
}) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const ref = item.stampRef!;
  const [meta, setMeta] = useState<StampMeta>({
    name: ref.name,
    emoji: item.emoji,
    keywords: item.keywords.join(" "),
  });
  const [confirmRemove, setConfirmRemove] = useState(false);
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "stamps" }));

  const save = useMutation({
    mutationFn: () =>
      studioClient().updateStamp({
        stampId: ref.id,
        name: meta.name.trim(),
        emoji: meta.emoji,
        keywords: { keywords: parseKeywords(meta.keywords) },
      }),
    onSuccess: async () => {
      // The answer carries only the stamp; the pack is read again so every copy of it agrees.
      await queryClient.invalidateQueries({ queryKey: ["stamp", "pack"] });
      onChanged(undefined);
      toast.success(t("toast.renamed"));
      onClose();
    },
    onError,
  });
  const cover = useMutation({
    mutationFn: () => studioClient().updatePack({ packId, coverStampId: ref.id }),
    onSuccess: (response) => {
      onChanged(response.pack);
      toast.success(t("toast.coverSet"));
      onClose();
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => studioClient().removeStamp({ stampId: ref.id }),
    onSuccess: (response) => {
      onChanged(response.pack);
      toast.success(t("toast.stampRemoved"));
      onClose();
    },
    onError,
  });

  return (
    <>
      <Dialog open onOpenChange={(open) => !open && onClose()} title={t("studio.pack.editStamp")}>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            save.mutate();
          }}
        >
          <div className="grid place-items-center rounded-[1.75rem] bg-surface-2 py-4">
            <StampImage stamp={ref} size={144} play="always" />
          </div>
          {item.pendingReview && !allPending && (
            <p className="rounded-2xl bg-sun px-3 py-2 text-center text-sm font-bold text-[#17122b]">
              {t("studio.pack.stampPending")}
            </p>
          )}
          {locked ? (
            <p className="text-center text-sm font-bold text-ink-soft">
              {t("studio.pack.lockedInReview")}
            </p>
          ) : (
            <StampMetaFields value={meta} onChange={setMeta} />
          )}
          {!locked && (
            <div className="flex flex-wrap justify-between gap-2">
              <div className="flex gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<Crown className="size-4" />}
                  disabled={isCover}
                  loading={cover.isPending}
                  onClick={() => cover.mutate()}
                >
                  {isCover ? t("studio.pack.isCover") : t("studio.pack.setCover")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-danger"
                  icon={<Trash2 className="size-4" />}
                  onClick={() => setConfirmRemove(true)}
                >
                  {t("studio.pack.removeStamp")}
                </Button>
              </div>
              <Button type="submit" loading={save.isPending}>
                {t("common:actions.save")}
              </Button>
            </div>
          )}
        </form>
      </Dialog>
      <ConfirmDialog
        open={confirmRemove}
        onOpenChange={setConfirmRemove}
        title={t("studio.pack.removeStampTitle")}
        description={t("studio.pack.removeStampBody")}
        confirmLabel={t("studio.pack.removeStamp")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate(undefined, { onSettled: () => setConfirmRemove(false) })}
      />
    </>
  );
}
