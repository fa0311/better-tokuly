import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { ask } from "@tokuly/api";
import { askKeys, charCount, QUESTION_MAX } from "@tokuly/core/ask";
import { Send } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { useBurst } from "#/features/engagement";
import { useTranslation } from "#/lib/use-translation";
import { boxClient } from "./api";
import { signature } from "./model";
import { Counter, ImageAttachmentField, Segmented, toastError, useImageAttachment } from "./parts";

export interface SendFormProps {
  /** Who receives it: named in the lines about what they will see. */
  recipientName: string;
  /** Whether a signature is accepted (a box can turn it off). */
  allowSigned: boolean;
  /** Whether an anonymous item would be accepted (the recipient needs a crowd to hide the sender in). */
  anonymousAvailable: boolean;
  allowImages: boolean;
  placeholder: string;
  submitLabel: string;
  initialText?: string;
  /** Sends it; the form clears itself once this resolves. */
  onSend(input: { text: string; signed: boolean; imageAssetId: string }): Promise<unknown>;
}

/**
 * The form for sending something to a person — a question to their box, a
 * response to their prompt: 300 characters, an optional picture, anonymous
 * unless the sender chooses to sign, and a plain line about what the
 * recipient will and will not see.
 */
export function SendForm({
  recipientName,
  allowSigned,
  anonymousAvailable,
  allowImages,
  placeholder,
  submitLabel,
  initialText = "",
  onSend,
}: SendFormProps) {
  const { t } = useTranslation("ask");
  const [text, setText] = useState(initialText);
  const [wantSigned, setWantSigned] = useState(false);
  const image = useImageAttachment();
  const { fire, burst } = useBurst(["💌", "✨", "💜"]);

  const [beganAnonymous] = useState(anonymousAvailable);
  const { choice, signed, allowed } = signature({
    allowSigned,
    anonymousAvailable,
    beganAnonymous,
    wantSigned,
  });
  const possible = anonymousAvailable || allowSigned;
  const length = charCount(text.trim());

  const send = useMutation({
    mutationFn: () => onSend({ text: text.trim(), signed, imageAssetId: image.assetId ?? "" }),
    onSuccess: () => {
      fire();
      setText("");
      image.clear();
    },
    onError: toastError,
  });

  if (!possible)
    return (
      <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
        {t("askForm.unavailable", { name: recipientName })}
      </p>
    );

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (allowed && length > 0 && length <= QUESTION_MAX && !image.busy) send.mutate();
      }}
    >
      <div className="rounded-control border-2 border-line bg-surface transition-colors focus-within:border-brand">
        <textarea
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={placeholder}
          aria-label={placeholder}
          rows={4}
          className="block w-full resize-none bg-transparent px-4 pt-3 text-[15px] leading-relaxed placeholder:text-ink-faint focus:outline-none"
        />
        <div className="flex items-end gap-2 px-2 pb-2">
          {allowImages && (
            <ImageAttachmentField
              attachment={image}
              addLabel={t("askForm.addImage")}
              removeLabel={t("askForm.removeImage")}
            />
          )}
          <span className="flex-1" />
          <span className="pr-2 pb-1">
            <Counter text={text.trim()} max={QUESTION_MAX} />
          </span>
        </div>
      </div>
      {allowImages && image.previewUrl && (
        <p className="px-1 text-xs text-ink-soft">{t("askForm.imageNote")}</p>
      )}

      {choice && (
        <Segmented
          label={t("askForm.anonymous")}
          value={wantSigned ? "signed" : "anonymous"}
          onChange={(value) => setWantSigned(value === "signed")}
          options={[
            {
              value: "anonymous",
              label: t("askForm.anonymous"),
              icon: <span aria-hidden>🙈</span>,
            },
            { value: "signed", label: t("askForm.signed"), icon: <span aria-hidden>🙋</span> },
          ]}
        />
      )}
      <div
        className="rounded-2xl bg-surface-2 px-4 py-3 text-sm leading-relaxed"
        data-testid="privacy-note"
      >
        {!anonymousAvailable && (
          <p className="mb-1 font-bold">{t("askForm.signedOnly", { name: recipientName })}</p>
        )}
        {signed ? (
          <p className="font-bold">{t("askForm.signedNote", { name: recipientName })}</p>
        ) : (
          allowed && (
            <>
              <p className="font-bold">{t("askForm.anonymousNote")}</p>
              <p className="mt-1 text-xs text-ink-soft">{t("askForm.anonymousNote2")}</p>
            </>
          )
        )}
      </div>

      <span className="relative self-stretch">
        <Button
          type="submit"
          block
          size="lg"
          icon={<Send className="size-5" />}
          loading={send.isPending}
          disabled={!allowed || length === 0 || length > QUESTION_MAX || image.busy}
        >
          {image.busy ? t("askForm.uploading") : submitLabel}
        </Button>
        {burst}
      </span>
    </form>
  );
}

/** Asking a question on someone's box, with the "sent!" moment afterwards. */
export function AskForm({ box, initialText }: { box: ask.QuestionBox; initialText?: string }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const [sent, setSent] = useState(false);
  const owner = box.owner!;

  if (sent) {
    return (
      <div
        className="flex animate-pop-in flex-col items-center gap-2 py-4 text-center"
        data-testid="ask-sent"
      >
        <div className="text-5xl">💌</div>
        <p className="text-lg font-black">{t("askForm.sent")}</p>
        <p className="text-sm text-ink-soft">{t("askForm.sentBody")}</p>
        <div className="mt-2 flex flex-wrap justify-center gap-2">
          <Button variant="secondary" size="sm" onClick={() => setSent(false)}>
            {t("askForm.another")}
          </Button>
          <Link
            to="/ask/sent"
            className="pressable sticker inline-flex h-9 items-center rounded-full bg-surface px-4 text-sm font-bold"
          >
            {t("askForm.seeSent")}
          </Link>
        </div>
      </div>
    );
  }
  return (
    <SendForm
      recipientName={owner.displayName}
      allowSigned={box.allowSigned}
      anonymousAvailable={box.anonymousAvailable}
      allowImages={box.allowImages}
      placeholder={t("askForm.placeholder")}
      submitLabel={t("askForm.send")}
      initialText={initialText}
      onSend={async (input) => {
        await boxClient().askQuestion({ user: owner.id, ...input });
        void queryClient.invalidateQueries({ queryKey: askKeys.asked });
        setSent(true);
      }}
    />
  );
}
