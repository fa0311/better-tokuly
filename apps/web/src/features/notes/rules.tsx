import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { notesKeys } from "@tokuly/core/notes";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { noteClient } from "./api";

/** The posting rules in the viewer's language, and whether they agreed to the current version. */
export function useNoteRules(enabled = true) {
  return useQuery({
    queryKey: notesKeys.rules,
    queryFn: () => noteClient().getNoteRules({}),
    enabled,
    staleTime: 5 * 60_000,
  });
}

const RULE_EMOJI = ["💜", "🔒", "📸", "🚫", "🎭", "🚩"];

/**
 * The rules everyone agrees to before their first note (and again when they
 * change). Shown in place of the editor until the viewer agrees.
 */
export function RulesAgreement({
  rules,
  error,
  onRetry,
  onCancel,
}: {
  rules:
    | notes.GetNoteRulesResponse
    | undefined /** Why the rules could not be read, if they could not. */;
  error?: unknown;
  onRetry?(): void;
  onCancel(): void;
}) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const agree = useMutation({
    mutationFn: () => noteClient().agreeToNoteRules({ version: rules!.version }),
    onSuccess: (response) => {
      queryClient.setQueryData<notes.GetNoteRulesResponse>(notesKeys.rules, (current) =>
        current
          ? {
              ...current,
              agreed: true,
              agreedVersion: current.version,
              agreedAt: response.agreedAt,
            }
          : current,
      );
    },
    onError: (error) => {
      // The rules changed while they were being read: show the new ones.
      void queryClient.invalidateQueries({ queryKey: notesKeys.rules });
      toast.error(errorMessage(error, { namespace: "notes" }));
    },
  });

  if (!rules && error) {
    return (
      <EmptyState
        emoji="🌧️"
        title={errorMessage(error, { namespace: "notes" })}
        action={<Button onClick={onRetry}>{t("timeline.retry")}</Button>}
      />
    );
  }
  if (!rules) {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-16" />
        <Skeleton className="h-16" />
        <Skeleton className="h-16" />
      </div>
    );
  }
  const updated = rules.agreedVersion > 0 && rules.agreedVersion < rules.version;
  return (
    <div data-testid="note-rules">
      <p className="text-sm leading-relaxed text-ink-soft">
        {updated ? t("rules.updated") : t("rules.lead")}
      </p>
      <ol className="mt-4 flex flex-col gap-2.5">
        {rules.rules.map((rule, index) => (
          <li key={rule.title} className="flex gap-3 rounded-2xl bg-surface-2 px-3 py-3">
            <span
              aria-hidden
              className="grid size-9 shrink-0 place-items-center rounded-xl border-2 border-ink bg-surface text-lg"
            >
              {RULE_EMOJI[index % RULE_EMOJI.length]}
            </span>
            <span className="min-w-0">
              <span className="block font-black">{rule.title}</span>
              <span className="block text-sm leading-relaxed text-ink-soft">{rule.body}</span>
            </span>
          </li>
        ))}
      </ol>
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={agree.isPending}>
          {t("rules.later")}
        </Button>
        <Button variant="lime" loading={agree.isPending} onClick={() => agree.mutate()}>
          {t("rules.agree")}
        </Button>
      </div>
    </div>
  );
}
