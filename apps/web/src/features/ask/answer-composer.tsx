import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ask, type common } from "@tokuly/api";
import { Globe, Lock, Smile, Users, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { imageUrl } from "#/features/notes";
import { StampImage, StampPicker } from "#/features/stamps";
import { boxClient, dropFromInbox, refreshAnswers } from "./api";
import { ANSWER_MAX, charCount } from "./model";
import {
  Counter,
  ImageAttachmentField,
  Picture,
  Segmented,
  Sender,
  toastError,
  useImageAttachment,
} from "./parts";

const AUDIENCES = {
  public: ask.AnswerAudience.PUBLIC,
  friends: ask.AnswerAudience.FRIENDS,
  private: ask.AnswerAudience.PRIVATE,
} as const;
type AudienceKey = keyof typeof AUDIENCES;
const audienceKey = (audience: ask.AnswerAudience | undefined): AudienceKey =>
  audience === ask.AnswerAudience.FRIENDS
    ? "friends"
    : audience === ask.AnswerAudience.PRIVATE
      ? "private"
      : "public";

/**
 * Answering a question of one's own box, or editing that answer: text, one
 * picture or one stamp, and who may read it. Rendered only while open, so
 * every opening starts from the question's current state.
 */
export function AnswerComposer({ question, onClose }: { question: ask.Question; onClose(): void }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const existing = question.answer;
  const [text, setText] = useState(existing?.text ?? "");
  const [audience, setAudience] = useState<AudienceKey>(audienceKey(existing?.audience));
  const [stamp, setStamp] = useState<common.StampRef | undefined>(existing?.stamp);
  // The picture already on the answer stays until it is taken out or replaced.
  const [keepImage, setKeepImage] = useState(Boolean(existing?.image));
  const image = useImageAttachment();

  const length = charCount(text.trim());
  const hasContent = length > 0 || Boolean(stamp) || keepImage || Boolean(image.assetId);

  const submit = useMutation({
    mutationFn: async () => {
      const body = {
        questionId: question.id,
        text: text.trim(),
        stampId: stamp?.id ?? "",
        audience: AUDIENCES[audience],
      };
      if (!existing)
        return (await boxClient().answerQuestion({ ...body, imageAssetId: image.assetId ?? "" }))
          .question!;
      // Unset keeps the picture that is there; empty takes it out.
      return (
        await boxClient().updateAnswer({
          ...body,
          imageAssetId: image.assetId ?? (keepImage ? undefined : ""),
        })
      ).question!;
    },
    onSuccess: (answered) => {
      if (!existing) dropFromInbox(queryClient, question.id);
      refreshAnswers(queryClient, answered);
      toast.success(existing ? t("answer.saved") : t("answer.done"));
      onClose();
    },
    onError: toastError,
  });

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={existing ? t("answer.editTitle") : t("answer.title")}
      footer={
        <Button
          block
          size="lg"
          loading={submit.isPending}
          disabled={!hasContent || length > ANSWER_MAX || image.busy}
          onClick={() => submit.mutate()}
        >
          {existing ? t("answer.save") : t("answer.submit")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="rounded-2xl bg-brand-soft px-4 py-3">
          <Sender anonymous={question.anonymous} user={question.asker} />
          <p className="mt-1 leading-snug font-black break-words whitespace-pre-wrap">
            {question.text}
          </p>
          {question.image && <Picture image={question.image} className="mt-2 max-w-40" />}
        </div>

        <div className="rounded-control border-2 border-line bg-surface transition-colors focus-within:border-brand">
          <textarea
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder={t("answer.placeholder")}
            aria-label={t("answer.placeholder")}
            rows={4}
            autoFocus
            className="block w-full resize-none bg-transparent px-4 pt-3 text-[15px] leading-relaxed placeholder:text-ink-faint focus:outline-none"
          />
          <div className="flex items-end gap-1 px-2 pb-2">
            {stamp ? (
              <span className="relative inline-block">
                <StampImage stamp={stamp} size={80} />
                <button
                  type="button"
                  aria-label={t("answer.removeAttachment")}
                  onClick={() => setStamp(undefined)}
                  className="absolute -top-2 -right-2 grid size-7 place-items-center rounded-full border-2 border-ink bg-surface"
                >
                  <X className="size-4" />
                </button>
              </span>
            ) : keepImage && existing?.image && !image.previewUrl ? (
              <span className="relative inline-block">
                <img
                  src={imageUrl(existing.image, 80)}
                  alt=""
                  className="size-20 rounded-2xl border-2 border-ink object-cover"
                />
                <button
                  type="button"
                  aria-label={t("answer.removeAttachment")}
                  onClick={() => setKeepImage(false)}
                  className="absolute -top-2 -right-2 grid size-7 place-items-center rounded-full border-2 border-ink bg-surface"
                >
                  <X className="size-4" />
                </button>
              </span>
            ) : (
              <>
                <ImageAttachmentField
                  attachment={image}
                  addLabel={t("answer.image")}
                  removeLabel={t("answer.removeAttachment")}
                />
                {!image.previewUrl && (
                  <StampPicker
                    trigger={
                      <button
                        type="button"
                        aria-label={t("answer.stamp")}
                        title={t("answer.stamp")}
                        className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
                      >
                        <Smile className="size-5" />
                      </button>
                    }
                    onPick={(picked) => {
                      // A picture and a stamp cannot go together.
                      image.clear();
                      setKeepImage(false);
                      setStamp(picked);
                    }}
                  />
                )}
              </>
            )}
            <span className="flex-1" />
            <span className="pr-2 pb-1">
              <Counter text={text.trim()} max={ANSWER_MAX} />
            </span>
          </div>
        </div>

        <div>
          <p className="mb-1.5 px-1 text-sm font-bold text-ink-soft">{t("answer.audience")}</p>
          <Segmented<AudienceKey>
            label={t("answer.audience")}
            value={audience}
            onChange={setAudience}
            options={[
              {
                value: "public",
                label: t("answer.audiences.public"),
                hint: t("answer.audiences.publicHint"),
                icon: <Globe className="size-4" />,
              },
              {
                value: "friends",
                label: t("answer.audiences.friends"),
                hint: t("answer.audiences.friendsHint"),
                icon: <Users className="size-4" />,
              },
              {
                value: "private",
                label: t("answer.audiences.private"),
                hint: t("answer.audiences.privateHint"),
                icon: <Lock className="size-4" />,
              },
            ]}
          />
        </div>
      </div>
    </Dialog>
  );
}
