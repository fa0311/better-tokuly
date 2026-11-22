import { Link } from "@tanstack/react-router";
import { type ask, toMillis } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { compactDuration } from "#/lib/format";

/** "あと3時間" while a prompt takes responses, "しめきり" afterwards. */
export function PromptState({ prompt, className }: { prompt: ask.Prompt; className?: string }) {
  const { t } = useTranslation("ask");
  const left = toMillis(prompt.closesAt) - Date.now();
  return prompt.open && left > 0 ? (
    <span
      className={cn(
        "rounded-full bg-lime px-2 py-0.5 text-[11px] font-black text-[#17122b]",
        className,
      )}
    >
      {t("prompts.closesIn", { time: compactDuration(left) })}
    </span>
  ) : (
    <span
      className={cn(
        "rounded-full bg-surface-3 px-2 py-0.5 text-[11px] font-black text-ink-soft",
        className,
      )}
    >
      {t("prompts.closed")}
    </span>
  );
}

/** A prompt in a list: its question, whose it is, and what waits behind it. Leads to the prompt's page. */
export function PromptCard({
  prompt,
  mine,
}: {
  prompt: ask.Prompt /** The viewer's own: counters instead of the owner's name. */;
  mine?: boolean;
}) {
  const { t } = useTranslation("ask");
  return (
    <Link
      to="/ask/prompts/$promptId"
      params={{ promptId: prompt.id }}
      className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-4"
      data-testid="prompt-card"
    >
      <span
        aria-hidden
        className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-2xl"
      >
        {prompt.emoji || "💭"}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block leading-snug font-black break-words">{prompt.text}</span>
        <span className="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
          {!mine && prompt.owner && (
            <span className="inline-flex min-w-0 items-center gap-1">
              <Avatar user={prompt.owner} size="xs" />
              <span className="truncate">
                <EmojiText text={prompt.owner.displayName} />
              </span>
            </span>
          )}
          <PromptState prompt={prompt} />
          {mine ? (
            <>
              <span>{t("prompts.responses", { count: prompt.responseCount })}</span>
              {prompt.unseenCount > 0 && (
                <span className="rounded-full bg-pink px-2 py-0.5 text-[11px] font-black text-white">
                  {t("prompts.unseen", { count: prompt.unseenCount })}
                </span>
              )}
              {prompt.publishedCount > 0 && (
                <span>{t("prompts.published", { count: prompt.publishedCount })}</span>
              )}
            </>
          ) : prompt.responded ? (
            <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-black">
              ✅ {t("prompts.responded")}
            </span>
          ) : (
            prompt.canRespond && (
              <span className="rounded-full bg-brand px-2 py-0.5 text-[11px] font-black text-on-brand">
                {t("prompts.respond")}
              </span>
            )
          )}
        </span>
      </span>
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </Link>
  );
}
