import { useMutation } from "@tanstack/react-query";
import type { common, emoji as emojiApi } from "@tokuly/api";
import { X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { errorMessage } from "#/lib/errors";
import { shortcode } from "../api";
import { draftReady, EMPTY_DRAFT, type EmojiDraft, EmojiForm } from "../emoji-form";
import { emojiAdminClient, UserLine, UserSearch, useAdminRefresh } from "./shared";

/** The longest reason the server keeps. */
const REASON_MAX = 500;

/** Asks for the reason of a rejection or of taking an emoji out of the catalogue; the author reads it. */
export function ReasonDialog({
  open,
  onOpenChange,
  title,
  confirmLabel,
  loading,
  onConfirm,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  confirmLabel: string;
  loading: boolean;
  onConfirm(reason: string): void;
}) {
  const { t } = useTranslation("emoji");
  const { t: tCommon } = useTranslation("common");
  const [reason, setReason] = useState("");
  // Each opening is a new decision: the reason of the last one is not carried over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setReason("");
  }
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={t("admin.reasonLead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={loading}>
            {tCommon("actions.cancel")}
          </Button>
          <Button
            variant="danger"
            loading={loading}
            disabled={!reason.trim()}
            onClick={() => onConfirm(reason.trim())}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <TextArea
        label={t("admin.reason")}
        value={reason}
        maxLength={REASON_MAX}
        onChange={(event) => setReason(event.target.value)}
        placeholder={t("admin.reasonPlaceholder")}
      />
    </Dialog>
  );
}

/**
 * A moderator's form for one emoji: adding one straight to the catalogue
 * (`detail` unset) or changing any field of an existing one, including who
 * is credited for it.
 */
export function EmojiEditDialog({
  detail,
  open,
  onOpenChange,
}: {
  detail?: emojiApi.EmojiDetail;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("emoji");
  const { t: tCommon } = useTranslation("common");
  const refresh = useAdminRefresh();
  const current = detail?.emoji;
  const [draft, setDraft] = useState<EmojiDraft>(() =>
    current
      ? {
          name: current.name,
          pictureUrl: current.image?.url,
          category: current.category,
          aliases: [...current.aliases],
          license: detail.license,
          sensitive: current.sensitive,
          localOnly: current.localOnly,
        }
      : EMPTY_DRAFT,
  );
  const [author, setAuthor] = useState<common.UserSummary | undefined>(
    detail?.author && !detail.author.deleted ? detail.author : undefined,
  );
  const [uploading, setUploading] = useState(false);

  const save = useMutation({
    mutationFn: async () => {
      const fields = {
        name: draft.name,
        category: draft.category,
        license: draft.license.trim(),
        sensitive: draft.sensitive,
        localOnly: draft.localOnly,
        authorId: author?.id ?? "",
      };
      const response = current
        ? await emojiAdminClient().updateEmoji({
            emojiId: current.id,
            ...fields,
            assetId: draft.assetId,
            aliases: { aliases: draft.aliases },
          })
        : await emojiAdminClient().addEmoji({
            ...fields,
            assetId: draft.assetId ?? "",
            aliases: draft.aliases,
          });
      return response.emoji;
    },
    onSuccess: async (saved) => {
      toast.success(
        t(current ? "admin.saved" : "admin.added", {
          code: shortcode(saved?.emoji?.name ?? draft.name),
        }),
      );
      onOpenChange(false);
      await refresh();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  const ready = draftReady(draft, uploading) && (Boolean(current) || Boolean(draft.assetId));

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={
        current ? t("admin.editTitle", { code: shortcode(current.name) }) : t("admin.addTitle")
      }
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={save.isPending}>
            {tCommon("actions.cancel")}
          </Button>
          <Button loading={save.isPending} disabled={!ready} onClick={() => save.mutate()}>
            {t(current ? "admin.save" : "admin.add")}
          </Button>
        </>
      }
    >
      <EmojiForm
        draft={draft}
        onChange={(patch) => setDraft((value) => ({ ...value, ...patch }))}
        onUploading={setUploading}
        currentName={current?.name}
        staff
      />
      <div className="mt-5">
        {author ? (
          <div>
            <p className="px-1 text-sm font-bold text-ink-soft">{t("admin.credit")}</p>
            <div className="mt-1.5 flex h-12 items-center gap-2 rounded-control border-2 border-line px-3 text-sm">
              <span className="min-w-0 flex-1 truncate">
                <UserLine user={author} />
              </span>
              <button
                type="button"
                aria-label={t("admin.removeCredit")}
                onClick={() => setAuthor(undefined)}
                className="grid size-7 shrink-0 place-items-center rounded-full hover:bg-surface-2"
              >
                <X className="size-4" />
              </button>
            </div>
          </div>
        ) : (
          <UserSearch label={t("admin.credit")} onPick={setAuthor} />
        )}
      </div>
    </Dialog>
  );
}
