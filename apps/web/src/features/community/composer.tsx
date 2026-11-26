import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, community, errorReason, fromDate } from "@tokuly/api";
import { can, communityKeys, parseTags, pseudonymName, upsertReply } from "@tokuly/core/community";
import { BarChart3, Plus, Smile, Sticker, Trash2, VenetianMask, X } from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker, EmojiSuggestBar, recordEmojiUse, shortcodesIn } from "#/features/emoji";
import { StampImage, StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { boardClient, NS } from "./api";
import {
  ImageAttachmentRow,
  ImagePickButton,
  Notice,
  useDurationLabel,
  useImageAttachments,
} from "./parts";
import { noteSlowPost, useSlowWait } from "./slow-mode";

/** A multi-line field with `:shortcode:` suggestions and an emoji picker. */
export function PostTextArea({
  value,
  onChange,
  label,
  placeholder,
  maxLength,
  rows = 4,
  toolbar,
  onSubmit,
}: {
  value: string;
  onChange(value: string): void;
  /** Accessible name of the field. */
  label: string;
  placeholder?: string;
  maxLength: number;
  rows?: number;
  /** Buttons next to the emoji picker (pictures, stamps). */
  toolbar?: ReactNode;
  /** Ctrl/Cmd+Enter. */
  onSubmit?(): void;
}) {
  const { t } = useTranslation(NS);
  const field = useRef<HTMLTextAreaElement>(null);
  const [caret, setCaret] = useState(0);
  const put = (text: string, at: number) => {
    onChange(text);
    setCaret(at);
    requestAnimationFrame(() => {
      field.current?.focus();
      field.current?.setSelectionRange(at, at);
    });
  };
  return (
    <div className="flex flex-col gap-1.5">
      <EmojiSuggestBar
        text={value}
        caret={caret}
        onComplete={(next) => put(next.text, next.caret)}
      />
      <textarea
        ref={field}
        aria-label={label}
        value={value}
        rows={rows}
        maxLength={maxLength}
        placeholder={placeholder}
        onChange={(event) => {
          onChange(event.target.value);
          setCaret(event.target.selectionStart);
        }}
        onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && onSubmit) {
            event.preventDefault();
            onSubmit();
          }
        }}
        className="w-full resize-y rounded-control border-2 border-line bg-surface px-4 py-3 text-[15px] leading-relaxed text-ink placeholder:text-ink-faint focus:border-brand focus:outline-none"
      />
      <div className="flex items-center gap-0.5">
        <EmojiPicker
          keepOpen
          onPick={(emoji) =>
            put(value.slice(0, caret) + emoji + value.slice(caret), caret + emoji.length)
          }
          trigger={
            <button
              type="button"
              aria-label={t("compose.emoji")}
              className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
            >
              <Smile className="size-5" />
            </button>
          }
        />
        {toolbar}
        <span className="ml-auto text-xs font-bold text-ink-faint">
          {value.length}/{maxLength}
        </span>
      </div>
    </div>
  );
}

function StampButton({
  onPick,
  disabled,
}: {
  onPick(stamp: common.StampRef): void;
  disabled?: boolean;
}) {
  const { t } = useTranslation(NS);
  return (
    <StampPicker
      onPick={onPick}
      trigger={
        <button
          type="button"
          disabled={disabled}
          aria-label={t("compose.stamp")}
          className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 disabled:opacity-40"
        >
          <Sticker className="size-5" />
        </button>
      }
    />
  );
}

function PickedStamp({ stamp, onRemove }: { stamp: common.StampRef; onRemove(): void }) {
  const { t } = useTranslation(NS);
  return (
    <div className="relative w-fit">
      <StampImage stamp={stamp} size={88} />
      <button
        type="button"
        onClick={onRemove}
        aria-label={t("compose.removeStamp")}
        className="absolute -top-2 -right-2 grid size-7 place-items-center rounded-full border-2 border-ink bg-surface"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}

/** What an anonymous channel means, said where someone is about to post in one. */
export function AnonymousNotice({ className }: { className?: string }) {
  const { t } = useTranslation(NS);
  return (
    <Notice icon={<VenetianMask />} className={className}>
      <p className="font-bold text-ink">{t("anonymous.title")}</p>
      <p>{t("anonymous.body")}</p>
    </Notice>
  );
}

/** Slow mode and the wait it leaves, for a composer. */
function SlowModeLine({ channel, wait }: { channel: community.Channel; wait: number }) {
  const { t } = useTranslation(NS);
  const durationLabel = useDurationLabel();
  if (channel.slowModeSeconds <= 0) return null;
  return (
    <p role="status" className="text-xs font-bold text-ink-faint">
      🐢{" "}
      {wait > 0
        ? t("slow.wait", { count: wait })
        : t("slow.on", { interval: durationLabel(channel.slowModeSeconds) })}
    </p>
  );
}

/** Whether the channel's slow mode holds this member back (the staff is exempt, except behind a pseudonym). */
function slowApplies(c: community.Community, channel: community.Channel): boolean {
  return (
    channel.slowModeSeconds > 0 &&
    (channel.anonymous || !can(c, community.Permission.MODERATE_CONTENT))
  );
}

/** Says what happened to a post the server took. */
function announcePost(
  t: (key: string) => string,
  state: community.PostState,
  what: "thread" | "reply",
) {
  if (state === community.PostState.HIDDEN) toast(t("compose.held"), { icon: "🕵️" });
  else toast.success(t(what === "thread" ? "compose.posted" : "compose.replied"));
}

const DEADLINES = [
  { value: "0", key: "none", hours: 0 },
  { value: "1", key: "hour", hours: 1 },
  { value: "24", key: "day", hours: 24 },
  { value: "72", key: "days3", hours: 72 },
  { value: "168", key: "week", hours: 168 },
] as const;

/** Starting a thread: where, a title, and at least one of text, pictures, a stamp or a poll. */
export function ThreadComposer({
  community: c,
  channels,
  channelId,
  open,
  onOpenChange,
  onCreated,
}: {
  community: community.Community;
  channels: community.Channel[];
  /** The channel the board is showing; empty = the first the person may post in. */
  channelId: string;
  open: boolean;
  onOpenChange(open: boolean): void;
  onCreated(thread: community.Thread): void;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const postable = channels.filter((channel) => channel.canPost);
  const [chosen, setChosen] = useState("");
  const channel =
    postable.find((each) => each.id === (chosen || channelId)) ?? postable[0] ?? undefined;
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [tags, setTags] = useState("");
  const [stamp, setStamp] = useState<common.StampRef>();
  const [poll, setPoll] = useState<{ options: string[]; multiple: boolean; hours: string }>();
  const attachments = useImageAttachments(4);
  const wait = useSlowWait(channel?.id);

  const reset = () => {
    setTitle("");
    setBody("");
    setTags("");
    setStamp(undefined);
    setPoll(undefined);
    attachments.reset();
  };

  const pollOptions = poll?.options.map((option) => option.trim()).filter(Boolean) ?? [];
  const pollReady =
    !poll || (pollOptions.length >= 2 && new Set(pollOptions).size === pollOptions.length);
  const hasContent =
    Boolean(body.trim()) || attachments.assetIds.length > 0 || Boolean(stamp) || Boolean(poll);
  const ready =
    Boolean(channel) && Boolean(title.trim()) && hasContent && pollReady && !attachments.uploading;

  const create = useMutation({
    mutationFn: () => {
      const hours = Number(poll?.hours ?? 0);
      return boardClient().createThread({
        channelId: channel?.id ?? "",
        title: title.trim(),
        body: body.trim(),
        assetIds: attachments.assetIds,
        stampId: stamp?.id ?? "",
        tags: parseTags(tags, 5),
        poll: poll
          ? {
              options: pollOptions,
              multipleChoice: poll.multiple,
              closesAt: hours > 0 ? fromDate(new Date(Date.now() + hours * 3_600_000)) : undefined,
            }
          : undefined,
      });
    },
    onSuccess: ({ thread }) => {
      if (!thread || !channel) return;
      recordEmojiUse(shortcodesIn(`${title} ${body}`));
      if (slowApplies(c, channel)) noteSlowPost(channel.id, channel.slowModeSeconds);
      void queryClient.invalidateQueries({ queryKey: communityKeys.threadLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.channels(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.followed });
      queryClient.setQueryData(communityKeys.thread(thread.shortId), thread);
      announcePost(t, thread.state, "thread");
      reset();
      onOpenChange(false);
      onCreated(thread);
    },
    onError: (error) => {
      if (errorReason(error) === "SLOW_MODE" && channel)
        noteSlowPost(channel.id, channel.slowModeSeconds);
      toast.error(errorMessage(error, { namespace: NS }));
    },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("compose.threadTitle")}
      footer={
        <Button
          loading={create.isPending}
          disabled={!ready || wait > 0}
          onClick={() => create.mutate()}
        >
          {wait > 0 ? t("slow.button", { count: wait }) : t("compose.post")}
        </Button>
      }
    >
      {!channel ? (
        <Notice>{t("compose.noChannel")}</Notice>
      ) : (
        <div className="flex flex-col gap-4">
          {postable.length > 1 && (
            <SelectField
              label={t("compose.channel")}
              value={channel.id}
              onChange={setChosen}
              options={postable.map((each) => ({
                value: each.id,
                label: `${each.emoji ? `${each.emoji} ` : ""}${each.name}`,
                hint: each.anonymous ? t("anonymous.chip") : undefined,
              }))}
            />
          )}
          {channel.anonymous && <AnonymousNotice />}
          <TextField
            label={t("compose.title")}
            value={title}
            maxLength={100}
            placeholder={t("compose.titlePlaceholder")}
            onChange={(event) => setTitle(event.target.value)}
          />
          <PostTextArea
            label={t("compose.body")}
            placeholder={t("compose.bodyPlaceholder")}
            value={body}
            onChange={setBody}
            maxLength={5000}
            rows={5}
            toolbar={
              <>
                <ImagePickButton
                  label={t("compose.addImage")}
                  disabled={attachments.full}
                  onPick={attachments.add}
                />
                <StampButton onPick={setStamp} />
                <button
                  type="button"
                  aria-label={t("poll.add")}
                  aria-pressed={Boolean(poll)}
                  onClick={() =>
                    setPoll(poll ? undefined : { options: ["", ""], multiple: false, hours: "0" })
                  }
                  className={cn(
                    "grid size-10 place-items-center rounded-full hover:bg-surface-2",
                    poll ? "text-brand" : "text-ink-soft",
                  )}
                >
                  <BarChart3 className="size-5" />
                </button>
              </>
            }
          />
          <ImageAttachmentRow images={attachments.images} onRemove={attachments.remove} />
          {stamp && <PickedStamp stamp={stamp} onRemove={() => setStamp(undefined)} />}
          {poll && (
            <fieldset className="flex flex-col gap-2 rounded-2xl border-2 border-line p-3">
              <legend className="px-1 text-sm font-black">{t("poll.title")}</legend>
              {poll.options.map((option, index) => (
                <div key={index} className="flex items-center gap-2">
                  <TextField
                    className="min-w-0 flex-1"
                    aria-label={t("poll.option", { index: index + 1 })}
                    placeholder={t("poll.option", { index: index + 1 })}
                    value={option}
                    maxLength={80}
                    onChange={(event) =>
                      setPoll({
                        ...poll,
                        options: poll.options.map((each, at) =>
                          at === index ? event.target.value : each,
                        ),
                      })
                    }
                  />
                  {poll.options.length > 2 && (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("poll.removeOption", { index: index + 1 })}
                      onClick={() =>
                        setPoll({ ...poll, options: poll.options.filter((_, at) => at !== index) })
                      }
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  )}
                </div>
              ))}
              {poll.options.length < 6 && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="self-start"
                  icon={<Plus className="size-4" />}
                  onClick={() => setPoll({ ...poll, options: [...poll.options, ""] })}
                >
                  {t("poll.addOption")}
                </Button>
              )}
              <SwitchRow
                label={t("poll.multiple")}
                checked={poll.multiple}
                onChange={(multiple) => setPoll({ ...poll, multiple })}
              />
              <SelectField
                label={t("poll.deadline")}
                value={poll.hours}
                onChange={(hours) => setPoll({ ...poll, hours })}
                options={DEADLINES.map(({ value, key }) => ({
                  value,
                  label: t(`poll.deadlines.${key}`),
                }))}
              />
              {!pollReady && <p className="px-1 text-xs text-danger">{t("poll.needOptions")}</p>}
            </fieldset>
          )}
          <TextField
            label={t("compose.tags")}
            hint={t("compose.tagsHint")}
            value={tags}
            placeholder="#テスト勉強 #雑談"
            onChange={(event) => setTags(event.target.value)}
          />
          <SlowModeLine channel={channel} wait={wait} />
        </div>
      )}
    </Dialog>
  );
}

/** The reply box under a thread: text, pictures or a stamp, optionally answering one reply. */
export function ReplyComposer({
  community: c,
  thread,
  channel,
  replyTo,
  onClearReplyTo,
}: {
  community: community.Community;
  thread: community.Thread;
  channel: community.Channel | undefined;
  replyTo: community.Reply | undefined;
  onClearReplyTo(): void;
}) {
  const { t, i18n } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [body, setBody] = useState("");
  const attachments = useImageAttachments(4);
  const wait = useSlowWait(thread.channelId);

  const send = useMutation({
    mutationFn: (stamp?: common.StampRef) =>
      boardClient().createReply({
        threadId: thread.id,
        body: stamp ? "" : body.trim(),
        assetIds: stamp ? [] : attachments.assetIds,
        stampId: stamp?.id ?? "",
        replyToId: replyTo?.id ?? "",
      }),
    onSuccess: ({ reply }, stamp) => {
      if (!reply) return;
      if (!stamp) {
        recordEmojiUse(shortcodesIn(body));
        setBody("");
        attachments.reset();
      }
      onClearReplyTo();
      if (channel && slowApplies(c, channel)) noteSlowPost(channel.id, channel.slowModeSeconds);
      upsertReply(queryClient, reply);
      void queryClient.invalidateQueries({ queryKey: communityKeys.thread(thread.shortId) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.thread(thread.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.followed });
      announcePost(t, reply.state, "reply");
    },
    onError: (error) => {
      if (errorReason(error) === "SLOW_MODE" && channel)
        noteSlowPost(channel.id, channel.slowModeSeconds);
      toast.error(errorMessage(error, { namespace: NS }));
    },
  });

  const ready = (Boolean(body.trim()) || attachments.assetIds.length > 0) && !attachments.uploading;
  const submit = () => {
    if (ready && wait === 0 && !send.isPending) send.mutate(undefined);
  };
  const quoted =
    replyTo &&
    (replyTo.pseudonym
      ? pseudonymName(replyTo.pseudonym, i18n.language)
      : replyTo.author?.displayName || replyTo.author?.handle || "");

  return (
    <section
      aria-label={t("compose.replyTitle")}
      className="sticker mt-4 flex flex-col gap-2 rounded-card bg-surface p-3"
    >
      {thread.pseudonym && (
        <p className="flex items-center gap-1.5 px-1 text-xs font-bold text-ink-soft">
          <VenetianMask className="size-4" />
          {t("anonymous.replyHint")}
        </p>
      )}
      {replyTo && (
        <div className="flex items-center gap-2 rounded-2xl bg-surface-2 px-3 py-2 text-sm">
          <span className="min-w-0 flex-1 truncate">
            {t("compose.replyingTo", { number: replyTo.number, name: quoted })}
          </span>
          <button type="button" onClick={onClearReplyTo} aria-label={t("compose.cancelReplyTo")}>
            <X className="size-4" />
          </button>
        </div>
      )}
      <PostTextArea
        label={t("compose.reply")}
        placeholder={t("compose.replyPlaceholder")}
        value={body}
        onChange={setBody}
        maxLength={2000}
        rows={2}
        onSubmit={submit}
        toolbar={
          <>
            <ImagePickButton
              label={t("compose.addImage")}
              disabled={attachments.full}
              onPick={attachments.add}
            />
            <StampButton
              disabled={wait > 0 || send.isPending}
              onPick={(stamp) => send.mutate(stamp)}
            />
          </>
        }
      />
      <ImageAttachmentRow images={attachments.images} onRemove={attachments.remove} />
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          {channel && <SlowModeLine channel={channel} wait={wait} />}
        </div>
        <Button size="sm" loading={send.isPending} disabled={!ready || wait > 0} onClick={submit}>
          {wait > 0 ? t("slow.button", { count: wait }) : t("compose.send")}
        </Button>
      </div>
    </section>
  );
}
