import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { refreshMembership, storeCommunity } from "@tokuly/core/community";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { communityClient, NS, useMyCommunities } from "./api";
import {
  CommunityFields,
  draftFields,
  draftQuestions,
  draftRules,
  draftTags,
  emptyDraft,
  useHandleCheck,
} from "./community-form";
import { Notice } from "./parts";

/** Starting a community: the person becomes its owner and lands on its front page. */
export function CreateCommunityScreen() {
  const { t } = useTranslation(NS);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(emptyDraft);
  const mine = useMyCommunities();
  const check = useHandleCheck(draft.handle);
  const atLimit = mine.maxOwned > 0 && mine.ownedCount >= mine.maxOwned;

  const create = useMutation({
    mutationFn: () =>
      communityClient().createCommunity({
        ...draftFields(draft),
        tags: draftTags(draft),
        rules: draftRules(draft),
        joinQuestions: draftQuestions(draft),
        iconAssetId: draft.icon?.assetId ?? "",
        bannerAssetId: draft.banner?.assetId ?? "",
      }),
    onSuccess: ({ community }) => {
      if (!community) return;
      storeCommunity(queryClient, community);
      refreshMembership(queryClient);
      toast.success(t("create.done"), { icon: "🎉" });
      void navigate({ to: "/g/$handle", params: { handle: community.handle } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const ready =
    Boolean(draft.name.trim()) && draft.handle.length >= 3 && check.settled && check.available;

  return (
    <Page title={t("create.title")} back="/communities">
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("create.lead")}</p>
      {atLimit && (
        <Notice tone="warn" className="mb-4">
          {t("create.limit", { count: mine.maxOwned })}
        </Notice>
      )}
      <CommunityFields draft={draft} onChange={setDraft} />
      <Button
        block
        size="lg"
        className="mt-6"
        loading={create.isPending}
        disabled={!ready || atLimit}
        onClick={() => create.mutate()}
      >
        {t("create.action")}
      </Button>
    </Page>
  );
}
