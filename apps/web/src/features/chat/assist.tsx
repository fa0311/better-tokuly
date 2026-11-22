import { useQuery } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import { Languages, Sparkles, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { create } from "zustand";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { assistClient } from "./api";
import { errorMessage } from "./errors";

/**
 * The AI writing assistant: rewrite the draft in another tone, suggest
 * replies, translate a message. Every use sends text to an outside AI
 * service, so nothing is sent before the person has been told and agreed
 * (once per browser), and never without them asking for it.
 */

const CONSENT_KEY = "tokuly.chat.assistConsent";

function hasConsent(): boolean {
  try {
    return window.localStorage.getItem(CONSENT_KEY) === "1";
  } catch {
    return false;
  }
}

interface AssistStore {
  /** What the person asked for before agreeing; runs once they do. */
  waiting: (() => void) | null;
  /** Messages whose translation is shown. */
  translated: Record<string, true>;
  /** Conversations for which reply ideas were asked: conversation → the message count they were asked at. */
  suggesting: Record<string, string>;
}

const useAssist = create<AssistStore>(() => ({ waiting: null, translated: {}, suggesting: {} }));

/** Runs `action` if the person agreed to the assistant's terms, asking them first otherwise. */
export function withAssistConsent(action: () => void): void {
  if (hasConsent()) action();
  else useAssist.setState({ waiting: action });
}

/** The one-time explanation of what the assistant sends where. Render once per screen that offers it. */
export function AssistConsent() {
  const { t } = useTranslation("chat");
  const waiting = useAssist((state) => state.waiting);
  return (
    <ConfirmDialog
      open={waiting !== null}
      onOpenChange={(open) => !open && useAssist.setState({ waiting: null })}
      title={t("assist.consent.title")}
      description={t("assist.consent.lead")}
      confirmLabel={t("assist.consent.agree")}
      onConfirm={() => {
        try {
          window.localStorage.setItem(CONSENT_KEY, "1");
        } catch {
          // Without storage the question simply comes again next time.
        }
        useAssist.setState({ waiting: null });
        waiting?.();
      }}
    >
      <ul className="flex list-disc flex-col gap-1.5 pl-5 text-sm text-ink-soft">
        <li>{t("assist.consent.rewrite")}</li>
        <li>{t("assist.consent.suggest")}</li>
        <li>{t("assist.consent.translate")}</li>
      </ul>
    </ConfirmDialog>
  );
}

// ───────────────────────────── Translate ─────────────────────────────

/** Shows (or hides again) the translation of a message. */
export function toggleTranslation(messageId: string): void {
  if (useAssist.getState().translated[messageId]) {
    useAssist.setState((state) => {
      const { [messageId]: _hidden, ...translated } = state.translated;
      return { translated };
    });
  } else {
    withAssistConsent(() =>
      useAssist.setState((state) => ({ translated: { ...state.translated, [messageId]: true } })),
    );
  }
}

export function useIsTranslated(messageId: string): boolean {
  return useAssist((state) => Boolean(state.translated[messageId]));
}

/** The translation of a message, under its bubble, once the person asked for it. */
export function TranslationBlock({
  message,
  tone,
}: {
  message: chat.Message;
  tone: "own" | "other";
}) {
  const { t, i18n } = useTranslation("chat");
  const shown = useIsTranslated(message.id);
  const translation = useQuery({
    queryKey: ["chat", "translation", message.id, i18n.language, message.text],
    queryFn: () =>
      assistClient().translateMessage({ messageId: message.id, targetLanguage: i18n.language }),
    enabled: shown,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
  if (!shown) return null;
  return (
    <div
      className={cn(
        "mt-1 max-w-full rounded-2xl border-2 border-dashed px-3 py-2 text-sm",
        tone === "own" ? "self-end" : "self-start",
        "border-line bg-surface text-ink",
      )}
      data-testid="translation"
    >
      <p className="mb-0.5 flex items-center gap-1 text-[11px] font-black text-brand">
        <Languages className="size-3.5" />
        {translation.data?.sourceLanguage
          ? t("assist.translatedFrom", {
              language: languageName(translation.data.sourceLanguage, i18n.language),
            })
          : t("assist.translation")}
        <button
          type="button"
          aria-label={t("assist.hideTranslation")}
          onClick={() => toggleTranslation(message.id)}
          className="ml-auto grid size-5 place-items-center rounded-full text-ink-faint hover:bg-surface-2"
        >
          <X className="size-3.5" />
        </button>
      </p>
      {translation.isPending ? (
        <Spinner className="size-4 text-ink-faint" />
      ) : translation.isError ? (
        <p className="text-danger">{errorMessage(translation.error)}</p>
      ) : (
        <p className="whitespace-pre-wrap break-words">{translation.data.text}</p>
      )}
    </div>
  );
}

function languageName(tag: string, displayIn: string): string {
  try {
    return new Intl.DisplayNames([displayIn], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

// ───────────────────────────── Rewrite and suggest ─────────────────────────────

const TONES: { tone: chat.RewriteTone; key: string }[] = [
  { tone: chat.RewriteTone.CASUAL, key: "casual" },
  { tone: chat.RewriteTone.POLITE, key: "polite" },
  { tone: chat.RewriteTone.CUTE, key: "cute" },
  { tone: chat.RewriteTone.FUNNY, key: "funny" },
  { tone: chat.RewriteTone.COOL, key: "cool" },
  { tone: chat.RewriteTone.EXCITED, key: "excited" },
  { tone: chat.RewriteTone.SHORTER, key: "shorter" },
  { tone: chat.RewriteTone.KANSAI, key: "kansai" },
];

/**
 * The ✨ button of the composer: rewrites what is typed in a chosen tone
 * (handing the result to `onRewritten`, which can be undone), or asks for
 * reply ideas when nothing is typed yet.
 */
export function AssistButton({
  conversationId,
  draft,
  lastSeq,
  onRewritten,
}: {
  conversationId: string;
  draft: string;
  lastSeq: bigint;
  onRewritten(text: string, original: string): void;
}) {
  const { t } = useTranslation("chat");
  const [busy, setBusy] = useState(false);
  const text = draft.trim();

  const rewrite = (tone: chat.RewriteTone) =>
    withAssistConsent(() => {
      setBusy(true);
      assistClient()
        .rewriteDraft({ text, tone })
        .then((response) => onRewritten(response.text, draft))
        .catch((error) => toast.error(errorMessage(error)))
        .finally(() => setBusy(false));
    });
  const suggest = () =>
    withAssistConsent(() =>
      useAssist.setState((state) => ({
        suggesting: { ...state.suggesting, [conversationId]: String(lastSeq) },
      })),
    );

  const actions: MenuAction[] = text
    ? TONES.map(({ tone, key }) => ({
        label: t(`assist.tone.${key}`),
        onSelect: () => rewrite(tone),
      }))
    : [{ label: t("assist.suggest"), icon: <Sparkles />, onSelect: suggest }];

  return (
    <Menu
      align="start"
      actions={actions}
      trigger={
        <button
          type="button"
          disabled={busy}
          aria-label={text ? t("assist.rewrite") : t("assist.title")}
          title={text ? t("assist.rewrite") : t("assist.title")}
          className="grid size-10 shrink-0 place-items-center rounded-full text-brand hover:bg-surface-2"
        >
          {busy ? <Spinner className="size-5" /> : <Sparkles className="size-5" />}
        </button>
      }
    />
  );
}

/** Reply ideas above the composer, after the person asked for them. Tapping one puts it in the draft. */
export function SuggestedReplies({
  conversationId,
  onPick,
}: {
  conversationId: string;
  onPick(text: string): void;
}) {
  const { t } = useTranslation("chat");
  const askedAt = useAssist((state) => state.suggesting[conversationId]);
  const close = () =>
    useAssist.setState((state) => {
      const { [conversationId]: _closed, ...suggesting } = state.suggesting;
      return { suggesting };
    });
  const replies = useQuery({
    queryKey: ["chat", "suggestions", conversationId, askedAt],
    queryFn: () => assistClient().suggestReplies({ conversationId }),
    enabled: askedAt !== undefined,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
  if (askedAt === undefined) return null;
  return (
    <div className="flex items-center gap-2 px-3 pb-2" data-testid="suggested-replies">
      <Sparkles className="size-4 shrink-0 text-brand" />
      <div className="scrollbar-none flex min-w-0 flex-1 gap-2 overflow-x-auto">
        {replies.isPending ? (
          <Spinner className="size-5 text-ink-faint" />
        ) : replies.isError ? (
          <p className="text-sm text-danger">{errorMessage(replies.error)}</p>
        ) : replies.data.replies.length === 0 ? (
          <p className="text-sm text-ink-soft">{t("assist.noSuggestions")}</p>
        ) : (
          replies.data.replies.map((reply) => (
            <Button
              key={reply}
              size="sm"
              variant="secondary"
              className="max-w-64 shrink-0"
              onClick={() => {
                onPick(reply);
                close();
              }}
            >
              <span className="truncate">{reply}</span>
            </Button>
          ))
        )}
      </div>
      <button
        type="button"
        aria-label={t("actions.close")}
        onClick={close}
        className="grid size-7 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
