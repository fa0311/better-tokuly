import { useMutation, useQueryClient } from "@tanstack/react-query";
import { DECK_MAX, type EmojiChoice, emojiKeys } from "@tokuly/core/emoji";
import { Plus, RotateCcw, Trash2, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { emojiClient, useDeck, useRecentEmoji } from "./api";
import { useCatalog } from "./catalog";
import { useSaveDeck } from "./deck";
import { EmojiImage } from "./emoji";
import { EmojiPicker, toChoices } from "./picker";

function Glyph({ choice, className }: { choice: EmojiChoice; className?: string }) {
  return choice.kind === "custom" ? (
    <EmojiImage emoji={choice.custom} size={36} className={cn("size-9", className)} />
  ) : (
    <span aria-hidden className={cn("text-[2rem] leading-none", className)}>
      {choice.emoji}
    </span>
  );
}

/**
 * "My reaction deck": the emoji offered first in every picker. Pin emoji,
 * drag them into order (or move them with the arrow keys), take them off;
 * below, what was used recently, one tap away from being pinned.
 */
export function DeckScreen() {
  const { t } = useTranslation("emoji");
  const queryClient = useQueryClient();
  const catalog = useCatalog();
  const deck = useDeck();
  const recent = useRecentEmoji();
  const save = useSaveDeck();
  const [resetting, setResetting] = useState(false);
  const [clearing, setClearing] = useState(false);

  const reset = useMutation({
    mutationFn: () => emojiClient().resetDeck({}),
    onSuccess: async () => {
      setResetting(false);
      toast.success(t("deck.resetDone"));
      await queryClient.invalidateQueries({ queryKey: emojiKeys.deck });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  const clear = useMutation({
    mutationFn: () => emojiClient().clearRecentEmoji({}),
    onSuccess: () => {
      setClearing(false);
      queryClient.setQueryData(emojiKeys.recent, []);
      toast.success(t("deck.recentCleared"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });

  const choices = toChoices(deck.data?.items ?? [], catalog);
  const codes = choices.map((choice) => choice.emoji);
  const full = codes.length >= DECK_MAX;
  const pin = (emoji: string) => {
    if (codes.includes(emoji)) return void toast(t("deck.already"));
    if (full) return void toast.error(t("errors:DECK_TOO_LONG"));
    save.mutate([...codes, emoji]);
  };
  const recentChoices = toChoices(recent.data ?? [], catalog);

  return (
    <Page
      title={t("deck.title")}
      back="/emoji"
      actions={
        <EmojiPicker
          side="bottom"
          keepOpen
          onPick={pin}
          trigger={
            <Button size="sm" icon={<Plus className="size-4" />} disabled={!deck.data}>
              {t("deck.add")}
            </Button>
          }
        />
      }
    >
      <p className="text-sm leading-relaxed text-ink-soft">{t("deck.lead")}</p>

      <section aria-label={t("deck.title")} className="mt-4 rounded-card bg-surface p-3">
        {deck.isPending ? (
          <Skeleton className="h-40" />
        ) : deck.isError ? (
          <EmptyState
            emoji="😵‍💫"
            title={t("browse.error")}
            body={errorMessage(deck.error, { namespace: "emoji" })}
          />
        ) : choices.length === 0 ? (
          <EmptyState emoji="📌" title={t("deck.empty")} body={t("deck.emptyBody")} />
        ) : (
          <Sortable
            items={choices}
            getKey={(choice) => choice.emoji}
            onReorder={(next) => save.mutate(next.map((choice) => choice.emoji))}
            grab="press"
            className="grid grid-cols-5 gap-2 sm:grid-cols-8"
          >
            {(choice, state) => (
              <>
                <button
                  type="button"
                  {...state.grip}
                  aria-label={t("deck.item", { emoji: choice.emoji })}
                  data-testid="deck-item"
                  data-emoji={choice.emoji}
                  className={cn(
                    "grid aspect-square w-full place-items-center rounded-2xl border-2 border-line bg-surface-2 outline-none focus-visible:border-brand",
                    state.dragging && "border-ink bg-brand-soft",
                  )}
                >
                  <Glyph choice={choice} />
                </button>
                <button
                  type="button"
                  aria-label={t("deck.remove", { emoji: choice.emoji })}
                  onClick={() => save.mutate(codes.filter((code) => code !== choice.emoji))}
                  className={cn(
                    "absolute -top-1.5 -right-1.5 grid size-6 place-items-center rounded-full border-2 border-ink bg-surface text-ink hover:bg-danger hover:text-white",
                    state.dragging && "invisible",
                  )}
                >
                  <X className="size-3.5" strokeWidth={3} />
                </button>
              </>
            )}
          </Sortable>
        )}
        {deck.data && (
          <div className="mt-3 flex flex-wrap items-center justify-between gap-2 px-1">
            <p className="text-xs font-bold text-ink-faint">
              {t("deck.count", { count: choices.length, max: DECK_MAX })}
              {deck.data.isDefault && ` · ${t("deck.isDefault")}`}
            </p>
            {!deck.data.isDefault && (
              <Button
                variant="ghost"
                size="sm"
                icon={<RotateCcw className="size-4" />}
                onClick={() => setResetting(true)}
              >
                {t("deck.reset")}
              </Button>
            )}
          </div>
        )}
      </section>
      {choices.length > 1 && (
        <p className="mt-2 px-1 text-xs text-ink-faint">{t("deck.dragHint")}</p>
      )}

      <section aria-label={t("deck.recent")} className="mt-6">
        <div className="flex items-center justify-between gap-2 px-1">
          <h2 className="font-black">{t("deck.recent")}</h2>
          {recentChoices.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              icon={<Trash2 className="size-4" />}
              onClick={() => setClearing(true)}
            >
              {t("deck.clearRecent")}
            </Button>
          )}
        </div>
        {recent.isPending ? (
          <Skeleton className="mt-2 h-16" />
        ) : recentChoices.length === 0 ? (
          <p className="mt-2 rounded-card bg-surface px-4 py-6 text-center text-sm text-ink-soft">
            {t("deck.recentEmpty")}
          </p>
        ) : (
          <>
            <div className="mt-2 grid grid-cols-6 gap-1.5 rounded-card bg-surface p-3 sm:grid-cols-10">
              {recentChoices.map((choice) => {
                const pinned = codes.includes(choice.emoji);
                return (
                  <button
                    key={choice.emoji}
                    type="button"
                    disabled={pinned}
                    aria-label={t("deck.pinThis", { emoji: choice.emoji })}
                    onClick={() => pin(choice.emoji)}
                    className="grid aspect-square place-items-center rounded-xl hover:bg-surface-2 disabled:opacity-35"
                  >
                    <Glyph
                      choice={choice}
                      className={choice.kind === "custom" ? "size-7" : "text-2xl"}
                    />
                  </button>
                );
              })}
            </div>
            <p className="mt-2 px-1 text-xs text-ink-faint">{t("deck.recentHint")}</p>
          </>
        )}
      </section>

      <ConfirmDialog
        open={resetting}
        onOpenChange={setResetting}
        title={t("deck.resetTitle")}
        description={t("deck.resetBody")}
        confirmLabel={t("deck.reset")}
        danger
        loading={reset.isPending}
        onConfirm={() => reset.mutate()}
      />
      <ConfirmDialog
        open={clearing}
        onOpenChange={setClearing}
        title={t("deck.clearTitle")}
        description={t("deck.clearBody")}
        confirmLabel={t("deck.clearRecent")}
        danger
        loading={clear.isPending}
        onConfirm={() => clear.mutate()}
      />
    </Page>
  );
}
