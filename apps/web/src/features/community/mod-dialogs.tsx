import { useMutation, useQuery } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { Eye } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { moderationClient, NS } from "./api";
import { Notice, RoleChip, useWhen } from "./parts";

/** A confirmation that takes a reason (deleting someone's post, banning, cancelling an event). */
export function ReasonDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  required,
  danger = true,
  loading,
  onConfirm,
  children,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  description?: string;
  confirmLabel: string;
  /** The reason must be given. */
  required?: boolean;
  danger?: boolean;
  loading?: boolean;
  onConfirm(reason: string): void;
  /** More fields above the reason (a duration). */
  children?: React.ReactNode;
}) {
  const { t } = useTranslation(NS);
  const [reason, setReason] = useState("");
  const [missing, setMissing] = useState(false);
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          setReason("");
          setMissing(false);
        }
        onOpenChange(next);
      }}
      title={title}
      description={description}
      confirmLabel={confirmLabel}
      danger={danger}
      loading={loading}
      onConfirm={() => {
        if (required && !reason.trim()) setMissing(true);
        else onConfirm(reason.trim());
      }}
    >
      <div className="flex flex-col gap-3">
        {children}
        <TextArea
          label={required ? t("reason.required") : t("reason.optional")}
          value={reason}
          maxLength={300}
          error={missing ? t("reason.missing") : undefined}
          onChange={(event) => {
            setReason(event.target.value);
            setMissing(false);
          }}
        />
      </div>
    </ConfirmDialog>
  );
}

/**
 * Looking up who wrote an anonymous post. The staff member gives a reason;
 * the lookup is logged and announced to the rest of the staff. The answer is
 * shown here once and kept nowhere: closing the dialog forgets it.
 */
export function RevealAuthorDialog({
  post,
  onClose,
}: {
  /** The post to look up; the dialog is open while this is set. */
  post: community.PostRef["post"] | undefined;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const [reason, setReason] = useState("");
  const reveal = useMutation({
    mutationFn: () =>
      moderationClient().revealAuthor({
        post: { post: post ?? { case: undefined } },
        reason: reason.trim(),
      }),
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const close = () => {
    setReason("");
    reveal.reset();
    onClose();
  };
  const result = reveal.data;
  return (
    <Dialog
      open={Boolean(post)}
      onOpenChange={(next) => !next && close()}
      title={t("reveal.title")}
      footer={
        result ? (
          <Button variant="secondary" onClick={close}>
            {t("close")}
          </Button>
        ) : (
          <Button
            icon={<Eye className="size-4" />}
            loading={reveal.isPending}
            disabled={!reason.trim()}
            onClick={() => reveal.mutate()}
          >
            {t("reveal.confirm")}
          </Button>
        )
      }
    >
      {result ? (
        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3 rounded-2xl bg-surface-2 p-3">
            <Avatar user={result.author} />
            <div className="min-w-0 flex-1">
              <p className="truncate font-bold">
                <EmojiText text={result.author?.displayName || result.author?.handle || "?"} />
              </p>
              <p className="truncate text-sm text-ink-soft">@{result.author?.handle}</p>
            </div>
            {result.role ? <RoleChip role={result.role} /> : null}
          </div>
          {!result.role && <p className="text-sm text-ink-soft">{t("reveal.notMember")}</p>}
          <Notice>{t("reveal.shownOnce")}</Notice>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <Notice tone="warn">{t("reveal.warning")}</Notice>
          <TextArea
            label={t("reveal.reason")}
            value={reason}
            maxLength={300}
            onChange={(event) => setReason(event.target.value)}
          />
        </div>
      )}
    </Dialog>
  );
}

/**
 * A thread, a reply or an event as the staff may read it: what it says now
 * (also after it was removed, while it is retained) and what it said before
 * each edit. Authors of anonymous posts stay pseudonyms.
 */
export function ContentHistoryDialog({
  content,
  onClose,
}: {
  content: community.ContentRef["content"] | undefined;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const key = content?.case ? `${content.case}:${content.value}` : "";
  const query = useQuery({
    queryKey: ["community", "history", key],
    queryFn: () =>
      moderationClient().getContentHistory({
        content: { content: content ?? { case: undefined } },
      }),
    enabled: Boolean(key),
    staleTime: 0,
    gcTime: 0,
  });
  const current = query.data?.current;
  const now =
    current?.case === "thread"
      ? { title: current.value.title, body: current.value.body }
      : current?.case === "reply"
        ? { title: "", body: current.value.body }
        : current?.case === "event"
          ? { title: current.value.title, body: current.value.description }
          : undefined;
  const removedAt = query.data?.removedAt ? Number(query.data.removedAt.seconds) * 1000 : 0;
  return (
    <Dialog
      open={Boolean(content)}
      onOpenChange={(next) => !next && onClose()}
      title={t("history.title")}
    >
      {query.isPending ? (
        <Skeleton className="h-32" />
      ) : query.error ? (
        <p className="text-sm text-danger">{errorMessage(query.error, { namespace: NS })}</p>
      ) : (
        <ol className="flex flex-col gap-3">
          <li className="rounded-2xl border-2 border-line p-3">
            <p className="text-xs font-black text-brand">
              {removedAt ? t("history.removedAt", { time: when(removedAt) }) : t("history.now")}
            </p>
            <Revision title={now?.title ?? ""} body={now?.body ?? ""} />
          </li>
          {[...(query.data?.revisions ?? [])].reverse().map((revision, index) => (
            <li key={index} className="rounded-2xl bg-surface-2 p-3">
              <p className="text-xs font-black text-ink-soft">
                {t("history.until", {
                  time: when(Number(revision.replacedAt?.seconds ?? 0n) * 1000),
                })}
              </p>
              <Revision title={revision.title} body={revision.body} />
            </li>
          ))}
          {query.data?.revisions.length === 0 && (
            <li className="text-sm text-ink-faint">{t("history.noEdits")}</li>
          )}
        </ol>
      )}
    </Dialog>
  );
}

function Revision({ title, body }: { title: string; body: string }) {
  const { t } = useTranslation(NS);
  return (
    <>
      {title && (
        <p className="mt-1 font-bold">
          <EmojiText text={title} />
        </p>
      )}
      <p className="mt-1 whitespace-pre-wrap break-words text-sm leading-relaxed">
        {body ? (
          <EmojiText text={body} />
        ) : (
          <span className="text-ink-faint">{t("history.noText")}</span>
        )}
      </p>
    </>
  );
}
