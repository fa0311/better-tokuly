import { useQueryClient } from "@tanstack/react-query";
import { chat, type common } from "@tokuly/api";
import {
  Camera,
  CornerUpLeft,
  FileText,
  ImagePlus,
  MapPin,
  Paperclip,
  Pencil,
  Play,
  Plus,
  Send,
  Smile,
  Sticker,
  X,
} from "lucide-react";
import {
  type ClipboardEvent,
  type KeyboardEvent,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmojiPicker, EmojiSuggestBar } from "#/features/emoji";
import { StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { realtime } from "#/lib/realtime";
import { AssistButton, SuggestedReplies } from "./assist";
import { describeFile, type OutgoingFile, releaseFiles } from "./attachments";
import { useConversationContext } from "./conversation-context";
import { loadDraft, saveDraft } from "./drafts";
import { errorMessage } from "./errors";
import { LocationShareDialog } from "./location-dialog";
import { useMembers } from "./members";
import { useMessageActions } from "./messages";
import { isGroup } from "./model";
import { send } from "./outbox";
import { useHover } from "./parts";
import { displayName } from "./people";
import { contentText } from "./preview";
import { canRecord } from "./recorder";
import { fileSize, mentionAt, playTime } from "./text";
import { VoiceRecorderControl } from "./voice-recorder";

/** While typing, the "is typing" signal is repeated this often (indicators expire after 6 s). */
const TYPING_EVERY_MS = 3_000;

interface MentionChoice {
  key: string;
  handle: string;
  user?: common.UserSummary;
}

/**
 * Where messages are written: text with mentions, stamps, emoji, photos,
 * videos and files (picked, pasted or dropped), voice messages, a place, and
 * — when the server offers it — the AI assistant. Also where a message is
 * answered (`replyTo`) or edited (`editing`).
 */
export function Composer({
  replyTo,
  onCancelReply,
  editing,
  onCancelEdit,
  onSent,
  onRequestPov,
  dropped,
}: {
  replyTo?: chat.Message;
  onCancelReply(): void;
  editing?: chat.Message;
  onCancelEdit(): void;
  /** Called after something was sent, so the screen can scroll to it. */
  onSent(): void;
  /** Opens "ask for a POV" (direct conversations between friends). */
  onRequestPov?(): void;
  /** Files dropped onto the conversation, to be attached. */
  dropped: File[];
}) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me, capabilities, writable } = useConversationContext();
  const actions = useMessageActions();
  const hover = useHover();
  const input = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const filePicker = useRef<HTMLInputElement>(null);
  const typedAt = useRef(0);
  /** Where the caret belongs once an inserted mention or emoji is in the field. */
  const caretAfterInsert = useRef<number | null>(null);
  const [text, setText] = useState(() => loadDraft(me.id, conversation.id));
  const [caret, setCaret] = useState(0);
  const [files, setFiles] = useState<OutgoingFile[]>([]);
  const [recording, setRecording] = useState(false);
  const [sharingPlace, setSharingPlace] = useState(false);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionsOff, setMentionsOff] = useState(false);
  const [saving, setSaving] = useState(false);

  const group = isGroup(conversation);
  const viewer = conversation.viewer;
  // The viewer's own message request, not accepted yet: text and stamps only, and only a few.
  const requestLeft = viewer?.requestMessagesLeft ?? -1;
  const limited = requestLeft >= 0;
  const trimmed = text.trim();
  const tooLong = text.length > capabilities.maxTextLength;
  // A caption may be edited away; any other message needs something in it.
  const hasContent = editing
    ? trimmed.length > 0 || editing.kind === chat.MessageKind.MEDIA
    : trimmed.length > 0 || files.length > 0;
  const canSend = hasContent && !tooLong && !saving;

  // ── Text: autosize, drafts, typing signals ──
  useLayoutEffect(() => {
    const element = input.current;
    if (!element) return;
    if (caretAfterInsert.current !== null) {
      element.focus();
      element.setSelectionRange(caretAfterInsert.current, caretAfterInsert.current);
      caretAfterInsert.current = null;
    }
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 148)}px`;
  }, [text]);

  useEffect(() => {
    if (editing) return;
    const timer = setTimeout(() => saveDraft(me.id, conversation.id, text), 400);
    return () => clearTimeout(timer);
  }, [text, editing, me.id, conversation.id]);

  // Editing takes over the field; the draft comes back afterwards.
  const editingId = editing?.id;
  useEffect(() => {
    setText(editing ? editing.text : loadDraft(me.id, conversation.id));
    if (editingId) input.current?.focus();
    // Only switching into or out of editing resets the field.
  }, [editingId]);

  useEffect(() => {
    if (replyTo) input.current?.focus();
  }, [replyTo]);

  const stopTyping = () => {
    if (typedAt.current === 0) return;
    typedAt.current = 0;
    realtime().sendTyping(conversation.id, false);
  };
  useEffect(() => stopTyping, []);

  const change = (value: string, position: number) => {
    setText(value);
    setCaret(position);
    setMentionsOff(false);
    if (editing) return;
    if (!value.trim()) stopTyping();
    else if (Date.now() - typedAt.current > TYPING_EVERY_MS) {
      typedAt.current = Date.now();
      realtime().sendTyping(conversation.id, true);
    }
  };

  // ── Mentions ──
  const mention = group && !mentionsOff ? mentionAt(text, caret) : null;
  const members = useMembers(conversation.id, mention !== null);
  const choices: MentionChoice[] = mention
    ? [
        ...("all".startsWith(mention.query.toLowerCase()) ? [{ key: "all", handle: "all" }] : []),
        ...(members.data?.pages ?? [])
          .flatMap((page) => page.members)
          .flatMap((member) =>
            member.user && member.user.id !== me.id && !member.user.deleted ? [member.user] : [],
          )
          .filter(
            (user) =>
              user.handle.toLowerCase().startsWith(mention.query.toLowerCase()) ||
              user.displayName.toLowerCase().includes(mention.query.toLowerCase()),
          )
          .map((user) => ({ key: user.id, handle: user.handle, user })),
      ].slice(0, 6)
    : [];
  const activeChoice = Math.min(mentionIndex, Math.max(0, choices.length - 1));

  const insertAt = (start: number, end: number, inserted: string) => {
    const next = text.slice(0, start) + inserted + text.slice(end);
    const position = start + inserted.length;
    // The caret goes behind what was inserted as soon as the new text is in the field (see the layout
    // effect): any later, and a key typed in between would land in the wrong place.
    caretAfterInsert.current = position;
    change(next, position);
  };
  const pickMention = (choice: MentionChoice) => {
    if (!mention) return;
    insertAt(mention.start, caret, `@${choice.handle} `);
    setMentionIndex(0);
  };
  const insertEmoji = (emoji: string) => {
    const element = input.current;
    insertAt(element?.selectionStart ?? text.length, element?.selectionEnd ?? text.length, emoji);
  };

  // ── Attachments ──
  const attach = async (picked: File[]) => {
    if (picked.length === 0) return;
    if (limited) {
      toast.error(t("composer.requestTextOnly"));
      return;
    }
    const room = capabilities.maxAttachments - files.length;
    if (picked.length > room)
      toast.error(t("composer.tooManyFiles", { count: capabilities.maxAttachments }));
    const described = await Promise.all(
      picked.slice(0, Math.max(0, room)).map((file) => describeFile(file)),
    );
    setFiles((current) => [...current, ...described]);
    input.current?.focus();
  };
  useEffect(() => {
    if (dropped.length > 0) void attach(dropped);
    // A new drop is the only trigger.
  }, [dropped]);

  // Files that were never sent are released when the composer goes away.
  const held = useRef(files);
  useEffect(() => {
    held.current = files;
  }, [files]);
  useEffect(() => () => releaseFiles(held.current), []);

  const onPaste = (event: ClipboardEvent) => {
    const pasted = Array.from(event.clipboardData.files);
    if (pasted.length === 0) return;
    event.preventDefault();
    void attach(pasted);
  };

  // ── Sending ──
  const deliver = (draft: Parameters<typeof send>[2]) => {
    send(queryClient, conversation.id, { ...draft, replyTo });
    if (replyTo) onCancelReply();
    onSent();
  };

  const submit = async () => {
    if (!canSend) return;
    if (editing) {
      if (trimmed === editing.text.trim()) {
        onCancelEdit();
        return;
      }
      setSaving(true);
      try {
        await actions.edit(editing.id, trimmed);
        onCancelEdit();
      } catch (error) {
        toast.error(errorMessage(error));
      } finally {
        setSaving(false);
      }
      return;
    }
    deliver({ text: trimmed, files });
    held.current = [];
    setFiles([]);
    setText("");
    saveDraft(me.id, conversation.id, "");
    typedAt.current = 0;
    input.current?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter while the keyboard is composing Japanese confirms the conversion; it is not "send".
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (choices.length > 0) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setMentionIndex(
          (activeChoice + (event.key === "ArrowDown" ? 1 : choices.length - 1)) % choices.length,
        );
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        pickMention(choices[activeChoice]!);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMentionsOff(true);
        return;
      }
    }
    if (event.key === "Escape" && (editing || replyTo)) {
      event.preventDefault();
      if (editing) onCancelEdit();
      else onCancelReply();
      return;
    }
    // With a keyboard, Enter sends and Shift+Enter breaks the line; on touch keyboards Enter breaks the line.
    if (event.key === "Enter" && !event.shiftKey && hover) {
      event.preventDefault();
      void submit();
    }
  };

  const pick = async (element: HTMLInputElement) => {
    const picked = Array.from(element.files ?? []);
    element.value = "";
    await attach(picked);
  };

  /** The assistant's version replaces the draft; the toast offers the way back. */
  const rewritten = (next: string, original: string) => {
    change(next, next.length);
    toast(t("assist.rewritten"), {
      action: { label: t("assist.undo"), onClick: () => change(original, original.length) },
    });
  };

  if (!writable) {
    return (
      <p className="border-t-2 border-line bg-surface px-4 pt-3 pb-safe text-center text-sm font-bold text-ink-soft">
        {t("composer.readOnly")}
      </p>
    );
  }
  if (limited && requestLeft === 0) {
    return (
      <p className="border-t-2 border-line bg-surface px-4 pt-3 pb-safe text-center text-sm font-bold text-ink-soft">
        {t("composer.requestWaiting")}
      </p>
    );
  }

  const more: MenuAction[] = [
    {
      label: t("composer.photo"),
      icon: <ImagePlus />,
      disabled: limited,
      onSelect: () => picker.current?.click(),
    },
    {
      label: t("composer.file"),
      icon: <Paperclip />,
      disabled: limited,
      onSelect: () => filePicker.current?.click(),
    },
    {
      label: t("composer.place"),
      icon: <MapPin />,
      disabled: limited,
      onSelect: () => setSharingPlace(true),
    },
    ...(onRequestPov
      ? [{ label: t("composer.pov"), icon: <Camera />, disabled: limited, onSelect: onRequestPov }]
      : []),
  ];
  const round =
    "grid size-10 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink";

  return (
    <div
      className="relative shrink-0 border-t-2 border-line bg-surface"
      onPaste={onPaste}
      data-testid="composer"
    >
      {/* `:` autocomplete: custom and Unicode emoji by name. */}
      <EmojiSuggestBar
        text={text}
        caret={caret}
        onComplete={(next) => {
          caretAfterInsert.current = next.caret;
          change(next.text, next.caret);
        }}
        className="absolute inset-x-3 bottom-full z-20 mb-2"
      />
      {choices.length > 0 && (
        <div
          role="listbox"
          aria-label={t("composer.mentions")}
          className="sticker absolute inset-x-3 bottom-full z-20 mb-2 flex flex-col overflow-hidden rounded-2xl bg-surface p-1.5 sm:right-auto sm:w-72"
        >
          {choices.map((choice, index) => (
            <button
              key={choice.key}
              type="button"
              role="option"
              aria-selected={index === activeChoice}
              // Keep the caret where it is: the text field must not lose focus.
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => pickMention(choice)}
              className={cn(
                "flex w-full items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left",
                index === activeChoice && "bg-surface-2",
              )}
            >
              {choice.user ? (
                <Avatar user={choice.user} size="sm" />
              ) : (
                <span className="grid size-8 place-items-center rounded-[0.6rem] bg-sun text-sm font-black text-[#17122b]">
                  @
                </span>
              )}
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-bold">
                  {choice.user ? displayName(choice.user) : t("composer.everyone")}
                </span>
                <span className="block truncate text-xs text-ink-soft">@{choice.handle}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      {(replyTo || editing) && (
        <div
          className="flex items-center gap-2 border-b-2 border-line px-3 py-2"
          data-testid={editing ? "editing-bar" : "reply-bar"}
        >
          {editing ? (
            <Pencil className="size-4 shrink-0 text-brand" />
          ) : (
            <CornerUpLeft className="size-4 shrink-0 text-brand" />
          )}
          <div className="min-w-0 flex-1 border-l-4 border-brand pl-2 text-xs">
            <p className="truncate font-black text-brand">
              {editing
                ? t("composer.editing")
                : t("composer.replyingTo", { name: displayName(replyTo?.sender) })}
            </p>
            <p className="truncate text-ink-soft">{contentText(editing ?? replyTo!, me.id)}</p>
          </div>
          <button
            type="button"
            aria-label={t("actions.cancel")}
            onClick={editing ? onCancelEdit : onCancelReply}
            className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
          >
            <X className="size-4" />
          </button>
        </div>
      )}

      {!editing && (
        <SuggestedReplies
          conversationId={conversation.id}
          onPick={(reply) => change(reply, reply.length)}
        />
      )}

      {files.length > 0 && (
        <ul
          className="scrollbar-none flex gap-2 overflow-x-auto px-3 pt-3"
          aria-label={t("composer.attachments")}
        >
          {files.map((file) => (
            <li key={file.id} className="relative shrink-0" data-testid="attachment-chip">
              <FileChip file={file} />
              <button
                type="button"
                aria-label={t("composer.removeFile", { name: file.name })}
                onClick={() => {
                  releaseFiles([file]);
                  setFiles((current) => current.filter((candidate) => candidate.id !== file.id));
                }}
                className="sticker-sm absolute -top-1.5 -right-1.5 grid size-6 place-items-center rounded-full bg-surface"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {limited && (
        <p className="px-4 pt-2 text-xs font-bold text-ink-soft">
          {t("composer.requestLeft", { count: requestLeft })}
        </p>
      )}

      <div className="flex items-end gap-1 px-2 pt-2 pb-safe">
        {recording ? null : editing ? null : (
          <>
            <Menu
              align="start"
              actions={more}
              trigger={
                <button type="button" aria-label={t("composer.more")} className={round}>
                  <Plus className="size-6" />
                </button>
              }
            />
            <StampPicker
              onPick={(stamp) => deliver({ text: "", stamp })}
              trigger={
                <button type="button" aria-label={t("composer.stamp")} className={round}>
                  <Sticker className="size-[22px]" />
                </button>
              }
            />
            {capabilities.assistEnabled && (
              <AssistButton
                conversationId={conversation.id}
                draft={text}
                lastSeq={conversation.lastSeq}
                onRewritten={rewritten}
              />
            )}
          </>
        )}
        {!recording && (
          <div className="relative flex min-w-0 flex-1 items-end rounded-[1.4rem] border-2 border-line bg-bg focus-within:border-brand">
            <textarea
              ref={input}
              rows={1}
              value={text}
              onChange={(event) => change(event.target.value, event.target.selectionStart)}
              onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
              onKeyDown={onKeyDown}
              onBlur={stopTyping}
              placeholder={editing ? t("composer.editPlaceholder") : t("composer.placeholder")}
              aria-label={t("composer.placeholder")}
              aria-invalid={tooLong || undefined}
              enterKeyHint={hover ? "send" : "enter"}
              className="max-h-[148px] min-h-[2.5rem] min-w-0 flex-1 resize-none bg-transparent py-2 pr-1 pl-4 text-[15px] leading-6 outline-none placeholder:text-ink-faint"
            />
            {hover && (
              <EmojiPicker
                keepOpen
                onPick={insertEmoji}
                trigger={
                  <button
                    type="button"
                    aria-label={t("emoji.title")}
                    className="mr-1 mb-0.5 grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
                  >
                    <Smile className="size-5" />
                  </button>
                }
              />
            )}
            {text.length > capabilities.maxTextLength - 200 && (
              <span
                className={cn(
                  "absolute -top-5 right-2 text-[11px] font-bold tabular-nums",
                  tooLong ? "text-danger" : "text-ink-faint",
                )}
              >
                {capabilities.maxTextLength - text.length}
              </span>
            )}
          </div>
        )}
        {canSend || editing || !canRecord() || limited ? (
          recording ? null : (
            <button
              type="button"
              aria-label={editing ? t("composer.save") : t("composer.send")}
              disabled={!canSend}
              onClick={() => void submit()}
              // Tapping "send" must not close the phone's keyboard.
              onMouseDown={(event) => event.preventDefault()}
              className="pressable sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-brand text-on-brand disabled:opacity-40"
            >
              <Send className="size-5" />
            </button>
          )
        ) : (
          <VoiceRecorderControl
            recording={recording}
            onRecordingChange={setRecording}
            onRecorded={(voice) => deliver({ text: "", files: [voice] })}
          />
        )}
      </div>

      <input
        ref={picker}
        type="file"
        accept="image/*,video/*"
        multiple
        className="hidden"
        data-testid="photo-input"
        onChange={(event) => void pick(event.target)}
      />
      <input
        ref={filePicker}
        type="file"
        multiple
        className="hidden"
        data-testid="file-input"
        onChange={(event) => void pick(event.target)}
      />
      <LocationShareDialog
        open={sharingPlace}
        onOpenChange={setSharingPlace}
        onShare={(location) => deliver({ text: "", location })}
      />
    </div>
  );
}

/** A file waiting in the composer: a thumbnail for pictures and videos, a labelled chip for the rest. */
function FileChip({ file }: { file: OutgoingFile }) {
  if (file.kind === "image")
    return (
      <img
        src={file.previewUrl}
        alt={file.name}
        className="size-16 rounded-2xl border-2 border-line object-cover"
      />
    );
  if (file.kind === "video") {
    return (
      <span className="relative block size-16 overflow-hidden rounded-2xl border-2 border-line bg-surface-3">
        <video
          src={`${file.previewUrl}#t=0.1`}
          muted
          playsInline
          preload="metadata"
          className="size-full object-cover"
        />
        <span className="absolute inset-0 grid place-items-center bg-black/20 text-white">
          <Play className="size-5 fill-current" />
        </span>
        {file.durationSeconds ? (
          <span className="absolute right-1 bottom-0.5 text-[10px] font-bold text-white tabular-nums">
            {playTime(file.durationSeconds)}
          </span>
        ) : null}
      </span>
    );
  }
  return (
    <span className="flex h-16 w-40 items-center gap-2 rounded-2xl border-2 border-line bg-surface-2 px-2.5">
      <FileText className="size-6 shrink-0 text-ink-soft" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-xs font-bold">{file.name}</span>
        <span className="block text-[11px] text-ink-soft">{fileSize(file.file.size)}</span>
      </span>
    </span>
  );
}
