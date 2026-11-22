import { Link } from "@tanstack/react-router";
import { ask, type common, media } from "@tokuly/api";
import { ImagePlus, X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Spinner } from "#/components/ui/spinner";
import { ImageViewer, imageUrl } from "#/features/notes";
import { StampImage } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { charCount } from "./model";

/** Shows API failures in this feature's own words. */
export const askError = (error: unknown): string => errorMessage(error, { namespace: "ask" });
export const toastError = (error: unknown): void => void toast.error(askError(error));

/** "12 / 300", turning red past the limit. */
export function Counter({ text, max }: { text: string; max: number }) {
  const count = charCount(text);
  return (
    <span
      className={cn(
        "text-xs font-bold whitespace-nowrap tabular-nums",
        count > max ? "text-danger" : "text-ink-faint",
      )}
    >{`${count} / ${max}`}</span>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  hint?: string;
  disabled?: boolean;
}

/** A row of choices where exactly one is on, each with its own line of explanation. */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: SegmentOption<T>[];
  value: T;
  onChange(value: T): void;
}) {
  const hint = options.find((option) => option.value === value)?.hint;
  return (
    <div>
      <div
        role="radiogroup"
        aria-label={label}
        className="flex gap-1 rounded-full bg-surface-2 p-1"
      >
        {options.map((option) => {
          const active = option.value === value;
          return (
            // biome-ignore lint/a11y/useSemanticElements: a styled radio group; the buttons carry the radio role.
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={option.disabled}
              onClick={() => onChange(option.value)}
              className={cn(
                "flex h-9 min-w-0 flex-1 items-center justify-center gap-1.5 rounded-full px-2 text-sm font-bold transition-colors disabled:opacity-40",
                active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink",
              )}
            >
              {option.icon}
              <span className="truncate">{option.label}</span>
            </button>
          );
        })}
      </div>
      {hint && <p className="mt-1.5 px-2 text-xs text-ink-soft">{hint}</p>}
    </div>
  );
}

/** A picture inside a question, an answer or a response; opens full screen on tap. */
export function Picture({
  image,
  className,
}: {
  image: common.Image | undefined;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  if (!image?.url) return null;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={cn("block overflow-hidden rounded-2xl border-2 border-line", className)}
      >
        <img
          src={imageUrl(image, 320)}
          alt=""
          loading="lazy"
          className="max-h-72 w-full object-cover"
          style={{
            background: image.dominantColor || undefined,
            aspectRatio:
              image.width && image.height ? `${image.width} / ${image.height}` : undefined,
          }}
        />
      </button>
      {open && <ImageViewer images={[image]} start={0} onClose={() => setOpen(false)} />}
    </>
  );
}

/** Who sent something: "匿名さん", or the person when they signed. */
export function Sender({
  anonymous,
  user,
  className,
}: {
  anonymous: boolean;
  user: common.UserSummary | undefined;
  className?: string;
}) {
  const { t } = useTranslation("ask");
  if (!anonymous && user) {
    return (
      <Link
        to="/u/$handle"
        params={{ handle: user.handle }}
        className={cn("inline-flex min-w-0 items-center gap-1.5 text-sm font-black", className)}
        data-testid="sender"
      >
        <Avatar user={user} size="xs" />
        <span className="truncate">{user.displayName}</span>
      </Link>
    );
  }
  return (
    <span
      className={cn("inline-flex items-center gap-1.5 text-sm font-black text-ink-soft", className)}
      data-testid="sender"
    >
      <span
        aria-hidden
        className="grid size-6 place-items-center rounded-[0.45rem] bg-surface-3 text-xs"
      >
        🙈
      </span>
      {t("anonymous")}
    </span>
  );
}

/** An answer's content: text, then its picture or stamp. */
export function AnswerBody({ answer, className }: { answer: ask.Answer; className?: string }) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {answer.text && (
        <p className="leading-relaxed break-words whitespace-pre-wrap">{answer.text}</p>
      )}
      {answer.image && <Picture image={answer.image} className="max-w-xs" />}
      {answer.stamp && <StampImage stamp={answer.stamp} size={120} />}
    </div>
  );
}

/** The label of an answer that is not for everyone. */
export function AudienceBadge({ audience }: { audience: ask.AnswerAudience }) {
  const { t } = useTranslation("ask");
  if (audience === ask.AnswerAudience.FRIENDS)
    return (
      <span className="rounded-full bg-sky px-2 py-0.5 text-[11px] font-black text-[#17122b]">
        👥 {t("answer.badge.friends")}
      </span>
    );
  if (audience === ask.AnswerAudience.PRIVATE)
    return (
      <span className="rounded-full bg-sun px-2 py-0.5 text-[11px] font-black text-[#17122b]">
        🔒 {t("answer.badge.private")}
      </span>
    );
  return null;
}

export interface ImageAttachment {
  /** A local preview while the file goes up, then of the upload. */
  previewUrl: string | undefined;
  /** Set once the upload can be attached. */
  assetId: string | undefined;
  busy: boolean;
  pick(file: File): void;
  clear(): void;
}

/**
 * One picture being attached to a question, a response or an answer. Each
 * pick is a new upload: the server refuses a picture that is in use anywhere
 * else (the same upload on two anonymous items would link them).
 */
export function useImageAttachment(): ImageAttachment {
  const { t } = useTranslation("ask");
  const [state, setState] = useState<{ previewUrl: string; assetId?: string } | null>(null);
  const controller = useRef<AbortController | null>(null);
  const preview = useRef<string | null>(null);

  const clear = () => {
    controller.current?.abort();
    controller.current = null;
    if (preview.current) URL.revokeObjectURL(preview.current);
    preview.current = null;
    setState(null);
  };
  // Leaving the form cancels an upload that is still on its way.
  useEffect(() => clear, []);

  const pick = (file: File) => {
    if (!file.type.startsWith("image/")) {
      toast.error(t("askForm.imageOnly"));
      return;
    }
    clear();
    const abort = new AbortController();
    controller.current = abort;
    const previewUrl = URL.createObjectURL(file);
    preview.current = previewUrl;
    setState({ previewUrl });
    uploadAndProcess(file, { purpose: media.AssetPurpose.NOTE_IMAGE, signal: abort.signal }).then(
      (asset) => !abort.signal.aborted && setState({ previewUrl, assetId: asset.id }),
      (error) => {
        if (abort.signal.aborted) return;
        clear();
        toastError(error);
      },
    );
  };

  return {
    previewUrl: state?.previewUrl,
    assetId: state?.assetId,
    busy: state !== null && !state.assetId,
    pick,
    clear,
  };
}

/** The "add a picture" button, or the picked picture with a button to take it out. */
export function ImageAttachmentField({
  attachment,
  addLabel,
  removeLabel,
}: {
  attachment: ImageAttachment;
  addLabel: string;
  removeLabel: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={input}
        type="file"
        accept="image/*"
        hidden
        data-testid="image-input"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file) attachment.pick(file);
          event.target.value = "";
        }}
      />
      {attachment.previewUrl ? (
        <span className="relative inline-block">
          <img
            src={attachment.previewUrl}
            alt=""
            className={cn(
              "size-20 rounded-2xl border-2 border-ink object-cover",
              attachment.busy && "opacity-50",
            )}
          />
          {attachment.busy && (
            <span className="absolute inset-0 grid place-items-center">
              <Spinner className="size-6" />
            </span>
          )}
          <button
            type="button"
            aria-label={removeLabel}
            onClick={attachment.clear}
            className="absolute -top-2 -right-2 grid size-7 place-items-center rounded-full border-2 border-ink bg-surface"
          >
            <X className="size-4" />
          </button>
        </span>
      ) : (
        <button
          type="button"
          aria-label={addLabel}
          title={addLabel}
          onClick={() => input.current?.click()}
          className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
        >
          <ImagePlus className="size-5" />
        </button>
      )}
    </>
  );
}

/** A section heading inside a page, with an optional control on the right. */
export function SectionTitle({
  children,
  action,
  className,
}: {
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("mt-7 mb-3 flex items-center gap-2 px-1", className)}>
      <h2 className="min-w-0 flex-1 truncate text-lg font-black">{children}</h2>
      {action}
    </div>
  );
}
