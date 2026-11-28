import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, engagement, media } from "@tokuly/api";
import { ChevronDown, Clock, ImagePlus, SendHorizontal, Smile, X } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { EmojiSuggestBar, EmojiText } from "#/features/emoji";
import { StampImage, StampPicker } from "#/features/stamps";
import { profileClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useMe } from "#/stores/session";
import { commentClient, updateReplies, updateThread, useCommentIdentities } from "./api";
import { channelFace } from "./author";
import { withComment, withPatched, withReply } from "./cache";
import { formatClock } from "./clock";
import { completeMention, mentionAtCaret } from "./text";
import { useThread } from "./thread-context";

const MAX_LENGTH = 1000;
const IDENTITY_KEY = "tokuly.comments.identity";

/** The channel the person last commented as in this browser ("" = as themselves). */
function storedIdentity(): string {
  try {
    return window.localStorage.getItem(IDENTITY_KEY) ?? "";
  } catch {
    return "";
  }
}

interface ImageDraft {
  /** Local preview while (and after) the file goes up. */
  previewUrl: string;
  /** Set once the upload is ready to attach. */
  assetId?: string;
  progress: number;
}

export interface ComposerProps {
  /** The comment being answered (a top-level comment or a reply); omit for a new top-level comment. */
  replyTo?: engagement.Comment;
  autoFocus?: boolean;
  /** Shown for replies: closes the composer without posting. */
  onCancel?(): void;
  onPosted?(comment: engagement.Comment): void;
}

/**
 * Writes a comment or a reply: text with `@mentions` (suggested while
 * typing), optionally one stamp or one image, a moment of the video where
 * there is one, and — for people with channels — which identity signs it.
 */
export function Composer({ replyTo, autoFocus, onCancel, onPosted }: ComposerProps) {
  const { t } = useTranslation("comments");
  const thread = useThread();
  const queryClient = useQueryClient();
  const me = useMe();
  const field = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  // Answering a reply starts with its author's handle, the way people expect a thread to read.
  const [text, setText] = useState(() =>
    replyTo?.parentId && replyTo.author?.user ? `@${replyTo.author.user.handle} ` : "",
  );
  const [active, setActive] = useState(Boolean(autoFocus));
  const [stamp, setStamp] = useState<common.StampRef | null>(null);
  const [image, setImage] = useState<ImageDraft | null>(null);
  const [moment, setMoment] = useState<number | null>(null);
  const [caret, setCaret] = useState(0);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionsDismissed, setMentionsDismissed] = useState(false);

  const identities = useCommentIdentities(Boolean(me));
  const [chosenChannel, setChosenChannel] = useState<string | null>(null);
  const preferred = chosenChannel ?? thread.defaultChannelId ?? storedIdentity();
  const channel = identities.data?.find((candidate) => candidate.id === preferred);

  const mention = mentionsDismissed ? null : mentionAtCaret(text, caret);
  const mentionQuery = mention?.query ?? "";
  const suggestions = useQuery({
    queryKey: ["identity", "search", "mention", mentionQuery],
    queryFn: async () =>
      (
        await profileClient().searchUsers({ query: mentionQuery, page: { limit: 6 } })
      ).results.flatMap((result) => (result.user ? [result.user] : [])),
    enabled: mentionQuery.length > 0,
    staleTime: 60_000,
  });
  const suggested = mention && mentionQuery ? (suggestions.data ?? []) : [];

  // The preview is a blob URL of this page: release it when the draft goes away.
  const previewUrl = image?.previewUrl;
  useEffect(() => () => void (previewUrl && URL.revokeObjectURL(previewUrl)), [previewUrl]);

  const post = useMutation({
    mutationFn: () =>
      commentClient().createComment({
        subject: thread.subject,
        parentId: replyTo?.id ?? "",
        text: text.trim(),
        stampId: stamp?.id ?? "",
        imageAssetId: image?.assetId ?? "",
        timestampSeconds: moment ?? undefined,
        channelId: channel?.id ?? "",
      }),
    onSuccess: ({ comment }) => {
      if (!comment) return;
      const published = comment.status === engagement.CommentStatus.PUBLISHED;
      if (comment.parentId) {
        updateReplies(queryClient, comment.parentId, (data) => withReply(data, comment, true));
        if (published)
          updateThread(queryClient, thread.subject, (data) =>
            withPatched(data, comment.parentId, (parent) => ({
              ...parent,
              replyCount: parent.replyCount + 1,
            })),
          );
      } else {
        updateThread(queryClient, thread.subject, (data) => withComment(data, comment, published));
      }
      if (!published) toast.message(t("composer.held"));
      setText("");
      setStamp(null);
      setImage(null);
      setMoment(null);
      setActive(false);
      onPosted?.(comment);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "comments" })),
  });

  const attachImage = async (file: File) => {
    const draft: ImageDraft = { previewUrl: URL.createObjectURL(file), progress: 0 };
    setStamp(null);
    setImage(draft);
    try {
      const asset = await uploadAndProcess(file, {
        purpose: media.AssetPurpose.COMMENT_IMAGE,
        onProgress: (progress) =>
          setImage((current) =>
            current?.previewUrl === draft.previewUrl ? { ...current, progress } : current,
          ),
      });
      setImage((current) =>
        current?.previewUrl === draft.previewUrl
          ? { ...current, assetId: asset.id, progress: 1 }
          : current,
      );
    } catch (error) {
      setImage((current) => (current?.previewUrl === draft.previewUrl ? null : current));
      toast.error(errorMessage(error, { namespace: "comments" }));
    }
  };

  const pickMention = (user: common.UserSummary) => {
    if (!mention) return;
    const next = completeMention(text, mention.start, caret, user.handle);
    setText(next.text);
    setCaret(next.caret);
    setMentionIndex(0);
    requestAnimationFrame(() => {
      field.current?.focus();
      field.current?.setSelectionRange(next.caret, next.caret);
    });
  };

  const uploading = Boolean(image && !image.assetId);
  const canPost =
    !uploading &&
    !post.isPending &&
    (text.trim().length > 0 || stamp !== null || Boolean(image?.assetId)) &&
    text.length <= MAX_LENGTH;
  const submit = () => {
    if (canPost) post.mutate();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggested.length > 0) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setMentionIndex(
          (index) =>
            (index + (event.key === "ArrowDown" ? 1 : suggested.length - 1)) % suggested.length,
        );
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        pickMention(suggested[Math.min(mentionIndex, suggested.length - 1)]!);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMentionsDismissed(true);
        return;
      }
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      submit();
    } else if (event.key === "Escape" && onCancel && !text.trim()) {
      onCancel();
    }
  };

  if (!me?.user) return null;
  const expanded = active || text.length > 0 || stamp !== null || image !== null;
  const face = channel ? channelFace(channel) : me.user;
  const nowAt = thread.video ? Math.floor(thread.video.currentTime()) : 0;

  return (
    <form
      className="flex items-start gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <Avatar user={face} size={replyTo ? "xs" : "sm"} className="mt-1" />
      <div className="relative min-w-0 flex-1">
        <div className="rounded-control border-2 border-line bg-surface transition-colors focus-within:border-brand">
          <textarea
            ref={field}
            value={text}
            // biome-ignore lint/a11y/noAutofocus: opened on purpose by tapping "reply" or "edit".
            autoFocus={autoFocus}
            rows={1}
            maxLength={MAX_LENGTH + 200}
            placeholder={replyTo ? t("composer.replyPlaceholder") : t("composer.placeholder")}
            aria-label={replyTo ? t("composer.replyPlaceholder") : t("composer.placeholder")}
            className="field-sizing-content block max-h-48 min-h-11 w-full resize-none bg-transparent px-4 py-2.5 text-[15px] leading-relaxed outline-none placeholder:text-ink-faint"
            onFocus={() => setActive(true)}
            onChange={(event) => {
              setText(event.target.value);
              setCaret(event.target.selectionStart);
              setMentionsDismissed(false);
              setMentionIndex(0);
            }}
            onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
            onKeyDown={onKeyDown}
          />

          {(stamp || image || moment !== null) && (
            <div className="flex flex-wrap items-end gap-2 px-3 pb-2">
              {stamp && (
                <Attachment label={t("composer.removeStamp")} onRemove={() => setStamp(null)}>
                  <StampImage stamp={stamp} size={72} />
                </Attachment>
              )}
              {image && (
                <Attachment label={t("composer.removeImage")} onRemove={() => setImage(null)}>
                  <img
                    src={image.previewUrl}
                    alt=""
                    className="h-20 max-w-40 rounded-xl object-cover"
                  />
                  {!image.assetId && (
                    <span
                      className="absolute inset-0 grid place-items-center rounded-xl bg-overlay text-white"
                      role="status"
                      aria-label={t("composer.uploading", {
                        percent: Math.round(image.progress * 100),
                      })}
                    >
                      <Spinner className="size-6" />
                    </span>
                  )}
                </Attachment>
              )}
              {moment !== null && (
                <span className="inline-flex h-8 items-center gap-1 rounded-full bg-brand-soft pr-1 pl-3 text-sm font-black text-brand-strong">
                  <Clock className="size-4" />
                  {formatClock(moment)}
                  <button
                    type="button"
                    aria-label={t("composer.removeTime")}
                    className="grid size-6 place-items-center rounded-full hover:bg-surface"
                    onClick={() => setMoment(null)}
                  >
                    <X className="size-3.5" />
                  </button>
                </span>
              )}
            </div>
          )}

          {expanded && (
            <div className="flex flex-wrap items-center gap-1 border-t-2 border-line px-2 py-1.5">
              <StampPicker
                trigger={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("composer.stamp")}
                    disabled={Boolean(image)}
                  >
                    <Smile className="size-5" />
                  </Button>
                }
                onPick={(picked) => {
                  setImage(null);
                  setStamp(picked);
                }}
              />
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("composer.image")}
                disabled={Boolean(stamp)}
                onClick={() => fileInput.current?.click()}
              >
                <ImagePlus className="size-5" />
              </Button>
              <input
                ref={fileInput}
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif"
                className="hidden"
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void attachImage(file);
                }}
              />
              {thread.video && moment === null && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="px-2.5"
                  icon={<Clock className="size-4" />}
                  onClick={() => setMoment(nowAt)}
                  aria-label={t("composer.addTime", { time: formatClock(nowAt) })}
                >
                  {formatClock(nowAt)}
                </Button>
              )}
              {(identities.data?.length ?? 0) > 0 && (
                <Menu
                  align="start"
                  trigger={
                    <Button
                      variant="ghost"
                      size="sm"
                      className="max-w-40 px-2.5"
                      aria-label={t("composer.identity")}
                    >
                      <span className="truncate">
                        {channel ? channel.name : t("composer.asMyself")}
                      </span>
                      <ChevronDown className="size-4 shrink-0" />
                    </Button>
                  }
                  actions={[
                    {
                      label: `${me.user.displayName} (@${me.user.handle})`,
                      icon: <Avatar user={me.user} size="xs" />,
                      onSelect: () => chooseIdentity(""),
                    },
                    ...(identities.data ?? []).map((candidate) => ({
                      label: `${candidate.name} (@${candidate.handle})`,
                      icon: <Avatar user={channelFace(candidate)} size="xs" />,
                      onSelect: () => chooseIdentity(candidate.id),
                    })),
                  ]}
                />
              )}
              {/* Kept together and to the right, also when the row wraps on a narrow phone. */}
              <div className="ml-auto flex items-center gap-1">
                {text.length > MAX_LENGTH - 100 && (
                  <span
                    className={cn(
                      "px-1 text-xs font-bold tabular-nums",
                      text.length > MAX_LENGTH ? "text-danger" : "text-ink-faint",
                    )}
                  >
                    {text.length}/{MAX_LENGTH}
                  </span>
                )}
                {onCancel && (
                  <Button variant="ghost" size="sm" onClick={onCancel}>
                    {t("composer.cancel")}
                  </Button>
                )}
                <Button
                  type="submit"
                  size="sm"
                  disabled={!canPost}
                  loading={post.isPending}
                  icon={<SendHorizontal className="size-4" />}
                >
                  {replyTo ? t("composer.reply") : t("composer.post")}
                </Button>
              </div>
            </div>
          )}
        </div>

        {/* `:` autocomplete: custom and Unicode emoji by name. */}
        <EmojiSuggestBar
          text={text}
          caret={caret}
          className="mt-1.5"
          onComplete={(next) => {
            setText(next.text);
            setCaret(next.caret);
            requestAnimationFrame(() => {
              field.current?.focus();
              field.current?.setSelectionRange(next.caret, next.caret);
            });
          }}
        />

        {suggested.length > 0 && (
          <div
            role="listbox"
            aria-label={t("composer.mentions")}
            className="sticker absolute inset-x-0 top-full z-30 mt-1 max-h-64 overflow-y-auto rounded-2xl bg-surface p-1.5"
          >
            {suggested.map((user, index) => (
              <div
                key={user.id}
                role="option"
                tabIndex={-1}
                aria-selected={index === mentionIndex}
                className={cn(
                  "flex cursor-pointer items-center gap-2.5 rounded-xl px-2.5 py-2",
                  index === mentionIndex && "bg-surface-2",
                )}
                // Keep the caret in the field: a mouse press would otherwise blur it first.
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => pickMention(user)}
                onMouseEnter={() => setMentionIndex(index)}
              >
                <Avatar user={user} size="sm" />
                <span className="min-w-0">
                  <span className="block truncate text-sm font-black">
                    <EmojiText text={user.displayName} />
                  </span>
                  <span className="block truncate text-xs text-ink-soft">@{user.handle}</span>
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </form>
  );

  function chooseIdentity(channelId: string) {
    setChosenChannel(channelId);
    try {
      window.localStorage.setItem(IDENTITY_KEY, channelId);
    } catch {
      // Private mode: the choice lasts for this page only.
    }
  }
}

function Attachment({
  label,
  onRemove,
  children,
}: {
  label: string;
  onRemove(): void;
  children: ReactNode;
}) {
  return (
    <span className="relative inline-block">
      {children}
      <button
        type="button"
        aria-label={label}
        className="sticker-sm absolute -top-2 -right-2 grid size-6 place-items-center rounded-full bg-surface"
        onClick={onRemove}
      >
        <X className="size-3.5" />
      </button>
    </span>
  );
}
