import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, emoji as emojiApi, errorReason, toDate } from "@tokuly/api";
import {
  Check,
  Copy,
  Eye,
  EyeOff,
  Pencil,
  Pin,
  PinOff,
  ShieldCheck,
  Smile,
  Trash2,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextArea } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { useStaffAccess } from "#/features/admin";
import { copyText, ReportButton } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { emojiKeys, shortcode, submissionClient, useDeck, useEmojiDetail } from "./api";
import { toCatalogEmoji } from "./catalog";
import { DECK_MAX, useSaveDeck } from "./deck";
import { EmojiImage } from "./emoji";
import { EmojiText } from "./emoji-text";
import { StateChip, ThemePreview, ThemeSwatch } from "./parts";
import { EmojiPicker } from "./picker";
import { useShowSensitive } from "./prefs";
import { completeShortcode, EmojiSuggestBar } from "./suggest-bar";

/** An emoji's page: the picture large, how it is written, who made it, and a place to try it in a sentence. */
export function DetailScreen({ name }: { name: string }) {
  const { t, i18n } = useTranslation("emoji");
  const detail = useEmojiDetail(name);
  const access = useStaffAccess();
  const showSensitive = useShowSensitive();
  // A sensitive emoji is blurred here too, until this visitor asks to see it.
  const [shown, setShown] = useState(false);

  if (detail.isPending) {
    return (
      <Page title={shortcode(name.replace(/:/g, ""))} back="/emoji">
        <Skeleton className="h-56" />
        <Skeleton className="mt-4 h-40" />
      </Page>
    );
  }
  const data = detail.data;
  if (detail.isError || !data?.emoji) {
    const missing = !detail.isError || errorReason(detail.error) === "EMOJI_NOT_FOUND";
    return (
      <Page title={t("title")} back="/emoji">
        <EmptyState
          emoji={missing ? "🫥" : "😵‍💫"}
          title={t(missing ? "detail.notFound" : "browse.error")}
          body={
            missing
              ? t("detail.notFoundBody", { code: shortcode(name.replace(/:/g, "")) })
              : errorMessage(detail.error)
          }
          action={
            <Link
              to="/emoji"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("detail.toCatalog")}
            </Link>
          }
        />
      </Page>
    );
  }

  const entry = toCatalogEmoji(data.emoji);
  const code = shortcode(entry.name);
  const approved = data.state === emojiApi.EmojiState.APPROVED;
  const added = toDate(data.emoji.addedAt) ?? toDate(data.createdAt);
  const dateFormat = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });

  return (
    <Page title={code} back="/emoji">
      <Card className="flex flex-col items-center gap-4 text-center">
        <div className="grid w-full grid-cols-2 gap-2">
          {(["light", "dark"] as const).map((theme) => (
            <ThemeSwatch key={theme} theme={theme} className="grid place-items-center py-5">
              <EmojiImage emoji={entry} size={128} reveal={shown} className="size-28" />
            </ThemeSwatch>
          ))}
        </div>
        {entry.sensitive && !showSensitive && (
          <Button
            variant="secondary"
            size="sm"
            aria-pressed={shown}
            icon={shown ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
            onClick={() => setShown(!shown)}
          >
            {t(shown ? "detail.hideSensitive" : "detail.showSensitive")}
          </Button>
        )}
        <div className="flex flex-wrap items-center justify-center gap-1.5 empty:hidden">
          {!approved && <StateChip state={data.state} />}
          {entry.animated && <Chip>{t("flags.animated")}</Chip>}
          {entry.sensitive && (
            <Chip className="bg-coral text-[#17122b]">{t("flags.sensitive")}</Chip>
          )}
          {entry.localOnly && <Chip>{t("flags.localOnly")}</Chip>}
        </div>
        <CopyCode code={code} />
        {!approved && data.reviewNote && (
          <p className="w-full rounded-2xl bg-surface-2 px-4 py-3 text-left text-sm">
            <span className="block text-xs font-black text-ink-faint">{t("submissions.note")}</span>
            {data.reviewNote}
          </p>
        )}
        <div className="flex flex-wrap items-center justify-center gap-2">
          {approved && <DeckButton code={code} />}
          {data.canEdit && <OwnerActions emojiId={entry.id} name={entry.name} />}
          {access.moderator && (
            <Link
              to="/admin/emoji"
              search={{ tab: "all", q: entry.name }}
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
            >
              <ShieldCheck className="size-4" />
              {t("detail.manage")}
            </Link>
          )}
        </div>
      </Card>

      {approved && <Playground code={code} />}

      <dl className="mt-4 divide-y-2 divide-line rounded-card bg-surface">
        <Fact label={t("fields.category")}>
          {entry.category ? (
            <Link
              to="/emoji"
              search={{ category: entry.category }}
              className="font-bold text-brand"
            >
              {entry.category}
            </Link>
          ) : (
            <span className="text-ink-faint">{t("detail.none")}</span>
          )}
        </Fact>
        <Fact label={t("fields.aliasesShort")}>
          {entry.aliases.length > 0 ? (
            <span className="flex flex-wrap gap-1.5">
              {entry.aliases.map((alias) => (
                <Chip key={alias}>{alias}</Chip>
              ))}
            </span>
          ) : (
            <span className="text-ink-faint">{t("detail.none")}</span>
          )}
        </Fact>
        <Fact label={t("detail.author")}>
          {data.author && !data.author.deleted ? (
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <Link
                to="/u/$handle"
                params={{ handle: data.author.handle }}
                className="inline-flex min-w-0 items-center gap-2 font-bold"
              >
                <Avatar user={data.author} size="xs" />
                <span className="truncate">
                  <EmojiText text={data.author.displayName} />
                </span>
              </Link>
              <Link
                to="/emoji"
                search={{ author: data.author.id }}
                className="text-sm font-bold text-brand"
              >
                {t("detail.moreByAuthor")}
              </Link>
            </span>
          ) : (
            <span className="text-ink-faint">{t("detail.noAuthor")}</span>
          )}
        </Fact>
        <Fact label={t("fields.licenseShort")}>
          {data.license ? (
            <span className="whitespace-pre-wrap break-words">{data.license}</span>
          ) : (
            <span className="text-ink-faint">{t("detail.none")}</span>
          )}
        </Fact>
        {added && (
          <Fact label={t(approved ? "detail.added" : "detail.submitted")}>
            {dateFormat.format(added)}
          </Fact>
        )}
        {/* The exact count is for moderators: emoji are used in private chats too. */}
        {access.moderator && (
          <Fact label={t("detail.uses")}>
            {t("detail.useCount", { count: Number(data.useCount) })}
          </Fact>
        )}
      </dl>

      {approved && (
        <div className="mt-3 flex justify-center">
          <ReportButton
            subject={{ type: common.SubjectType.CUSTOM_EMOJI, id: entry.id }}
            label={code}
          />
        </div>
      )}
    </Page>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline gap-3 px-4 py-3 text-[15px]">
      <dt className="w-24 shrink-0 text-xs font-black text-ink-faint">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  );
}

function CopyCode({ code }: { code: string }) {
  const { t } = useTranslation("emoji");
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    if (!(await copyText(code))) return void toast.error(t("detail.copyFailed"));
    setCopied(true);
    toast.success(t("detail.copied", { code }));
    setTimeout(() => setCopied(false), 1600);
  };
  return (
    <button
      type="button"
      onClick={copy}
      aria-label={t("detail.copy", { code })}
      className="pressable sticker inline-flex h-12 max-w-full items-center gap-2 rounded-full bg-surface-2 px-5 font-mono text-lg font-bold"
    >
      <span className="truncate">{code}</span>
      {copied ? (
        <Check className="size-5 shrink-0 text-success" />
      ) : (
        <Copy className="size-5 shrink-0 text-ink-soft" />
      )}
    </button>
  );
}

/** Pins the emoji to the person's reaction deck, or takes it off. */
function DeckButton({ code }: { code: string }) {
  const { t } = useTranslation("emoji");
  const deck = useDeck();
  const save = useSaveDeck();
  const items = deck.data?.items.map((item) => item.emoji) ?? [];
  const pinned = items.includes(code);
  const toggle = () => {
    if (!pinned && items.length >= DECK_MAX) return void toast.error(t("errors.DECK_TOO_LONG"));
    save.mutate(pinned ? items.filter((item) => item !== code) : [...items, code], {
      onSuccess: () => toast.success(t(pinned ? "deck.unpinned" : "deck.pinned")),
    });
  };
  return (
    <Button
      variant={pinned ? "secondary" : "lime"}
      disabled={!deck.data}
      aria-pressed={pinned}
      icon={pinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
      onClick={toggle}
    >
      {t(pinned ? "deck.unpin" : "deck.pin")}
    </Button>
  );
}

/** What the author may do while their emoji waits or was declined: change it, or take it back. */
function OwnerActions({ emojiId, name }: { emojiId: string; name: string }) {
  const { t } = useTranslation("emoji");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const withdraw = useMutation({
    mutationFn: () => submissionClient().withdrawSubmission({ emojiId }),
    onSuccess: async () => {
      toast.success(t("submissions.withdrawn"));
      await queryClient.invalidateQueries({ queryKey: emojiKeys.all });
      await navigate({ to: "/emoji/submissions" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  return (
    <>
      <Link
        to="/emoji/submissions/$emojiId"
        params={{ emojiId }}
        className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
      >
        <Pencil className="size-4" />
        {t("submissions.edit")}
      </Link>
      <Button
        variant="ghost"
        className="text-danger"
        icon={<Trash2 className="size-4" />}
        onClick={() => setConfirming(true)}
      >
        {t("submissions.withdraw")}
      </Button>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("submissions.withdrawTitle", { code: shortcode(name) })}
        description={t("submissions.withdrawBody")}
        confirmLabel={t("submissions.withdraw")}
        danger
        loading={withdraw.isPending}
        onConfirm={() => withdraw.mutate()}
      />
    </>
  );
}

/**
 * "Try it": a line of text with the emoji in it, drawn the way a message
 * would be, on both themes. Typing `:` and a few letters suggests emoji, and
 * the picker inserts one at the caret.
 */
function Playground({ code }: { code: string }) {
  const { t } = useTranslation("emoji");
  const [text, setText] = useState(() => t("detail.trySample", { code }));
  const [caret, setCaret] = useState(text.length);
  const fieldId = useId();

  const moveCaret = (position: number) => {
    setCaret(position);
    requestAnimationFrame(() => {
      const field = document.getElementById(fieldId) as HTMLTextAreaElement | null;
      field?.focus();
      field?.setSelectionRange(position, position);
    });
  };
  const insert = (emoji: string, at: number) => {
    const next = completeShortcode(text, at, at, emoji);
    setText(next.text);
    moveCaret(next.caret);
  };

  return (
    <section aria-label={t("detail.try")} className="mt-4 rounded-card bg-surface p-4">
      <h2 className="flex items-center gap-2 font-black">
        <Smile className="size-5 text-brand" />
        {t("detail.try")}
      </h2>
      <p className="mt-0.5 text-sm text-ink-soft">{t("detail.tryBody")}</p>
      <div className="relative mt-3">
        <TextArea
          id={fieldId}
          aria-label={t("detail.tryLabel")}
          value={text}
          maxLength={280}
          onChange={(event) => {
            setText(event.target.value);
            setCaret(event.target.selectionStart);
          }}
          onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
        />
        <div className="absolute right-1.5 bottom-1.5">
          <EmojiPicker
            keepOpen
            onPick={(emoji) => insert(emoji, caret)}
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("picker.open")}>
                <Smile className="size-5" />
              </Button>
            }
          />
        </div>
      </div>
      <EmojiSuggestBar
        text={text}
        caret={caret}
        className="mt-2"
        onComplete={(next) => {
          setText(next.text);
          moveCaret(next.caret);
        }}
      />
      <ThemePreview className="mt-3">
        <p className="whitespace-pre-wrap break-words" data-testid="emoji-text-preview">
          <EmojiText text={text} />
        </p>
      </ThemePreview>
    </section>
  );
}
