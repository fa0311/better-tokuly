import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { social } from "@tokuly/api";
import { X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { FollowButton } from "./follow-button";
import { followClient, followKeys } from "./follow-data";
import { SectionTitle, Tag, UserRow } from "./parts";

/** The reasons this screen has words for; anything else (a future signal) is not shown. */
const REASONS = [
  "FOLLOWS_YOU",
  "FRIEND_OF_FRIEND",
  "FOLLOWED_BY_FOLLOWING",
  "POPULAR_AT_SCHOOL",
  "POPULAR_IN_COMMUNITY",
] as const;

const known = (reason: string): reason is (typeof REASONS)[number] =>
  (REASONS as readonly string[]).includes(reason);

/** おすすめアカウント: people to follow, each with why, follow and "not interested". */
export function FollowSuggestions() {
  const { t } = useTranslation("follows");
  const queryClient = useQueryClient();
  const suggestions = useQuery({
    queryKey: followKeys.suggestions,
    queryFn: () => followClient().listFollowSuggestions({ limit: 20 }),
  });
  const dismiss = useMutation({
    mutationFn: (userId: string) => followClient().dismissFollowSuggestion({ userId }),
    onMutate: (userId) =>
      queryClient.setQueryData<social.ListFollowSuggestionsResponse>(
        followKeys.suggestions,
        (data) =>
          data && {
            ...data,
            suggestions: data.suggestions.filter((suggestion) => suggestion.user?.id !== userId),
          },
      ),
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "follows" }));
      return queryClient.invalidateQueries({ queryKey: followKeys.suggestions });
    },
  });

  const rows = suggestions.data?.suggestions ?? [];
  if (rows.length === 0) return null;
  return (
    <section data-testid="follow-suggestions">
      <SectionTitle>{t("suggestions.title")}</SectionTitle>
      {rows.map(
        (suggestion) =>
          suggestion.user && (
            <UserRow
              key={suggestion.user.id}
              testId="follow-suggestion"
              user={suggestion.user}
              subtitle={<Reasons suggestion={suggestion} />}
              trailing={
                <span className="flex items-center gap-1">
                  <FollowButton user={suggestion.user} relation={suggestion.relation} />
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("suggestions.dismiss", { name: suggestion.user.displayName })}
                    onClick={() => dismiss.mutate(suggestion.user!.id)}
                  >
                    <X className="size-4" />
                  </Button>
                </span>
              }
            />
          ),
      )}
    </section>
  );
}

function Reasons({ suggestion }: { suggestion: social.FollowSuggestion }) {
  const { t } = useTranslation("follows");
  const reasons = suggestion.reasons.filter(known).slice(0, 2);
  const first = suggestion.followedBy[0];
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      {reasons.map((reason) => (
        <Tag key={reason}>{t(`suggestions.reasons.${reason}`)}</Tag>
      ))}
      {first && (
        <span className="truncate text-xs">
          {suggestion.followedByCount > 1
            ? t("suggestions.followedByMany", {
                name: first.displayName,
                count: suggestion.followedByCount - 1,
              })
            : t("suggestions.followedBy", { name: first.displayName })}
        </span>
      )}
      {reasons.length === 0 && !first && (
        <span className="truncate">@{suggestion.user?.handle}</span>
      )}
    </span>
  );
}
