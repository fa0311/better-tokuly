import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  formFromDraft,
  formProblem,
  type PlanFormValue,
  rememberPlan,
  toCreateRequest,
} from "@tokuly/core/plans";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/button";
import { Page } from "#/components/page";
import { useFriends } from "#/features/friends/data";
import { asViewer, showPlanError } from "./actions";
import { usePlanZone } from "./data";
import { PlanForm } from "./plan-form";

/**
 * A new plan, empty or with a friend already invited (`inviteId`, from their
 * profile; `inviteHandle` finds them among more friends than one page
 * holds). Sent, it opens in place of the form.
 */
export function PlanNewScreen({
  inviteId,
  inviteHandle = "",
}: {
  inviteId?: string;
  inviteHandle?: string;
}) {
  const { t } = useTranslation("plans");
  const zone = usePlanZone();
  const queryClient = useQueryClient();
  const friends = useFriends(inviteHandle);
  const [edited, setEdited] = useState<PlanFormValue | null>(null);
  const [tried, setTried] = useState(false);

  // Until the person types, the form follows the draft: the friend appears once the friends are known.
  const friendUsers = (friends.data?.friends ?? []).flatMap((friend) =>
    friend.user ? [friend.user] : [],
  );
  const value =
    edited ?? formFromDraft({ inviteeIds: inviteId ? [inviteId] : [] }, friendUsers, zone);
  const problem = formProblem(value, zone);

  const create = useMutation({
    mutationFn: () => asViewer((service) => service.createPlan(toCreateRequest(value, zone))),
    onSuccess: (response) => {
      const plan = response?.plan;
      if (!plan) return;
      rememberPlan(queryClient, plan);
      router.replace(`/plans/${plan.id}`);
    },
    onError: showPlanError,
  });

  return (
    <Page testID="plan-new">
      <PlanForm
        value={value}
        onChange={setEdited}
        withInvitees
        problem={tried && problem ? t(`plan.form.errors.${problem}`) : null}
      />
      <Button
        testID="plan-create"
        size="lg"
        block
        loading={create.isPending}
        onPress={() => {
          setTried(true);
          if (!problem) create.mutate();
        }}
      >
        {t("plan.create")}
      </Button>
    </Page>
  );
}
