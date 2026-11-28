import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { askKeys, charCount } from "@tokuly/core/ask";
import { Plus, SmilePlus } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { EmojiPicker } from "#/features/engagement";
import { useTranslation } from "#/lib/use-translation";
import { promptClient, useFriendPrompts, usePromptsOf } from "./api";
import { PROMPT_MAX } from "./model";
import { Counter, SectionTitle, toastError } from "./parts";
import { PromptCard } from "./prompt-card";

const DURATIONS = {
  h1: 3600,
  h6: 6 * 3600,
  h24: 24 * 3600,
  d3: 3 * 86_400,
  d7: 7 * 86_400,
} as const;
type DurationKey = keyof typeof DURATIONS;

/** Prompts (お題): the ones friends are collecting responses for, and the viewer's own. */
export function PromptsScreen() {
  const { t } = useTranslation("ask");
  const friends = useFriendPrompts();
  const mine = usePromptsOf("");
  const [creating, setCreating] = useState(false);
  const friendPrompts = friends.data?.pages.flatMap((page) => page.prompts) ?? [];
  const myPrompts = mine.data?.pages.flatMap((page) => page.prompts) ?? [];

  return (
    <Page
      title={t("prompts.title")}
      back="/ask"
      wide
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("prompts.create")}
        </Button>
      }
    >
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("prompts.lead")}</p>
      <div className="grid grid-cols-1 items-start gap-x-8 lg:grid-cols-2">
        <section>
          <SectionTitle>{t("prompts.friends")}</SectionTitle>
          {friends.isPending ? (
            <Skeleton className="h-20" />
          ) : friendPrompts.length === 0 ? (
            <p className="rounded-card bg-surface px-4 py-6 text-center text-sm font-bold text-ink-soft">
              🍃 {t("prompts.friendsEmpty")}
            </p>
          ) : (
            <div className="flex flex-col gap-3" data-testid="friend-prompts">
              {friendPrompts.map((prompt) => (
                <PromptCard key={prompt.id} prompt={prompt} />
              ))}
              <LoadMore query={friends} />
            </div>
          )}
        </section>
        <section>
          <SectionTitle>{t("prompts.mine")}</SectionTitle>
          {mine.isPending ? (
            <Skeleton className="h-20" />
          ) : myPrompts.length === 0 ? (
            <EmptyState
              emoji="💭"
              title={t("prompts.mineEmpty")}
              body={t("prompts.mineEmptyBody")}
              action={
                <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
                  {t("prompts.create")}
                </Button>
              }
            />
          ) : (
            <div className="flex flex-col gap-3" data-testid="my-prompts">
              {myPrompts.map((prompt) => (
                <PromptCard key={prompt.id} prompt={prompt} mine />
              ))}
              <LoadMore query={mine} />
            </div>
          )}
        </section>
      </div>
      {creating && <CreatePromptDialog onClose={() => setCreating(false)} />}
    </Page>
  );
}

function CreatePromptDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [text, setText] = useState("");
  const [emoji, setEmoji] = useState("💭");
  const [duration, setDuration] = useState<DurationKey>("h24");
  const ideas = t("prompts.form.ideas", { returnObjects: true }) as string[];
  const length = charCount(text.trim());

  const create = useMutation({
    mutationFn: async () =>
      (
        await promptClient().createPrompt({
          text: text.trim(),
          emoji,
          durationSeconds: DURATIONS[duration],
        })
      ).prompt!,
    onSuccess: (prompt) => {
      void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
      toast.success(t("prompts.form.created"));
      onClose();
      void navigate({ to: "/ask/prompts/$promptId", params: { promptId: prompt.id } });
    },
    onError: toastError,
  });

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("prompts.form.title")}
      footer={
        <Button
          block
          size="lg"
          loading={create.isPending}
          disabled={length === 0 || length > PROMPT_MAX}
          onClick={() => create.mutate()}
        >
          {t("prompts.form.submit")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-start gap-3">
          <EmojiPicker
            side="bottom"
            onPick={setEmoji}
            trigger={
              <button
                type="button"
                aria-label={t("prompts.form.emoji")}
                className="pressable sticker grid size-14 shrink-0 place-items-center rounded-2xl bg-sun text-3xl"
              >
                {emoji || <SmilePlus className="size-6 text-[#17122b]" />}
              </button>
            }
          />
          <TextArea
            className="min-w-0 flex-1"
            aria-label={t("prompts.form.text")}
            placeholder={t("prompts.form.textPlaceholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
            rows={2}
            hint={<Counter text={text.trim()} max={PROMPT_MAX} />}
          />
        </div>
        <div className="flex flex-wrap gap-2">
          {ideas.map((idea) => (
            <button
              key={idea}
              type="button"
              onClick={() => setText(idea)}
              className="rounded-full bg-surface-2 px-3 py-1.5 text-sm font-bold text-ink-soft hover:bg-surface-3 hover:text-ink"
            >
              {idea}
            </button>
          ))}
        </div>
        <SelectField<DurationKey>
          label={t("prompts.form.duration")}
          value={duration}
          onChange={setDuration}
          options={(Object.keys(DURATIONS) as DurationKey[]).map((value) => ({
            value,
            label: t(`prompts.form.durations.${value}`),
          }))}
        />
      </div>
    </Dialog>
  );
}
