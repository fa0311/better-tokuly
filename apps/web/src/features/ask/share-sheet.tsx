import type { ask } from "@tokuly/api";
import { Download, Link2, Send, Share2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { ShareToChat } from "#/features/chat";
import { copyText } from "#/features/engagement";
import { loadNow } from "#/lib/load-module";
import type { Card } from "./card-image";
import { answerShareUrl, boxShareUrl } from "./model";

/** What a share sheet shows: everything of a card except the QR choice, which the sheet offers itself. */
export type ShareCard =
  | Omit<Extract<Card, { kind: "box" }>, "withQr">
  | Extract<Card, { kind: "answer" }>;

/**
 * The share sheet of a question box or of one answer: the link, and an image
 * card for stories that can be saved, handed to the device's share sheet or
 * sent to a friend in chat.
 */
export function ShareSheet({
  open,
  onOpenChange,
  card,
  title,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  card: ShareCard;
  title: string;
}) {
  const { t } = useTranslation("ask");
  const [withQr, setWithQr] = useState(true);
  const [image, setImage] = useState<{ blob: Blob; url: string } | null>(null);
  const [failed, setFailed] = useState(false);
  const [toChat, setToChat] = useState(false);
  // The card as one value: a new picture is drawn only when something on it changed.
  const spec = JSON.stringify(card.kind === "box" ? { ...card, withQr } : card);

  useEffect(() => {
    if (!open) return;
    let current = true;
    let url: string | undefined;
    setFailed(false);
    // The canvas code (and the QR encoder) is only loaded when someone shares.
    loadNow(() => import("./card-image"))
      .then(({ renderCard }) => renderCard(JSON.parse(spec) as Card))
      .then((blob) => {
        if (!current) return;
        url = URL.createObjectURL(blob);
        setImage({ blob, url });
      })
      .catch(() => current && setFailed(true));
    return () => {
      current = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [open, spec]);

  const file = () =>
    image ? new File([image.blob], `tokuly-${card.handle}.png`, { type: "image/png" }) : undefined;
  const canShareImage =
    typeof navigator !== "undefined" &&
    typeof navigator.canShare === "function" &&
    Boolean(image) &&
    navigator.canShare({ files: [file()!] });

  const copy = async () => {
    if (await copyText(card.url)) toast.success(t("share.copied"));
    else toast.error(t("share.copyFailed"));
  };
  const save = () => {
    if (!image) return;
    const link = document.createElement("a");
    link.href = image.url;
    link.download = `tokuly-${card.handle}.png`;
    link.click();
    toast.success(t("share.saved"));
  };
  const shareImage = async () => {
    try {
      await navigator.share({ files: [file()!], text: t("share.shareText", { url: card.url }) });
    } catch {
      // Closing the device's share sheet is not a failure.
    }
  };

  return (
    <>
      <Dialog
        open={open && !toChat}
        onOpenChange={onOpenChange}
        title={title}
        description={t("share.lead")}
      >
        <div className="flex flex-col gap-4 sm:flex-row">
          <div className="mx-auto w-44 shrink-0 sm:w-48">
            {image ? (
              <img
                src={image.url}
                alt={t("share.imageAlt")}
                data-testid="share-card"
                className="sticker aspect-[9/16] w-full rounded-2xl object-cover"
              />
            ) : failed ? (
              <p className="grid aspect-[9/16] place-items-center rounded-2xl bg-surface-2 p-3 text-center text-sm font-bold text-ink-soft">
                {t("share.imageFailed")}
              </p>
            ) : (
              <Skeleton className="aspect-[9/16] w-full" />
            )}
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-2.5">
            <p
              className="truncate rounded-2xl bg-surface-2 px-3 py-2.5 text-sm font-bold text-ink-soft"
              data-testid="share-url"
            >
              {card.url}
            </p>
            <Button block icon={<Link2 className="size-4" />} onClick={copy}>
              {t("share.copy")}
            </Button>
            {canShareImage && (
              <Button
                block
                variant="lime"
                icon={<Share2 className="size-4" />}
                onClick={shareImage}
              >
                {t("share.shareImage")}
              </Button>
            )}
            <Button
              block
              variant="secondary"
              icon={<Download className="size-4" />}
              disabled={!image}
              onClick={save}
            >
              {t("share.save")}
            </Button>
            <Button
              block
              variant="secondary"
              icon={<Send className="size-4" />}
              onClick={() => setToChat(true)}
            >
              {t("share.toChat")}
            </Button>
            {card.kind === "box" && (
              <SwitchRow label={t("share.qr")} checked={withQr} onChange={setWithQr} />
            )}
          </div>
        </div>
      </Dialog>
      {toChat && (
        <ShareToChat
          open
          onOpenChange={(next) => {
            setToChat(next);
            if (!next) onOpenChange(false);
          }}
          share={{ url: card.url }}
          title={card.kind === "box" ? card.prompt : card.question}
        />
      )}
    </>
  );
}

/** The share sheet of a question box. */
export function BoxShareSheet({
  open,
  onOpenChange,
  handle,
  prompt,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  handle: string;
  prompt: string;
}) {
  const { t } = useTranslation("ask");
  return (
    <ShareSheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("share.title")}
      card={{
        kind: "box",
        label: t("share.cardLabel"),
        prompt: prompt || t("settings.promptPlaceholder"),
        handle,
        callToAction: t("share.cardCta"),
        url: boxShareUrl(handle),
      }}
    />
  );
}

/** The share sheet of one published answer. */
export function AnswerShareSheet({
  open,
  onOpenChange,
  question,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  question: ask.Question;
}) {
  const { t } = useTranslation("ask");
  const handle = question.owner?.handle ?? "";
  return (
    <ShareSheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("share.answerTitle")}
      card={{
        kind: "answer",
        label:
          !question.anonymous && question.asker
            ? t("share.answerCardLabelSigned", { name: question.asker.displayName })
            : t("share.answerCardLabelAnonymous"),
        question: question.text,
        answer: question.answer?.text ?? "",
        handle,
        callToAction: t("share.answerCardCta"),
        url: answerShareUrl(handle, question.id),
      }}
    />
  );
}
