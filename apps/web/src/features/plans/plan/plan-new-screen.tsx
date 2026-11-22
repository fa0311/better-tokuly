import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { planClient, rememberPlan, useFriends } from "../api";
import { type PlanDraft, takePlanDraft } from "../draft";
import {
  EMPTY_FORM,
  formFromDraft,
  formProblem,
  type PlanFormValue,
  toCreateRequest,
} from "./model";
import { PlanForm } from "./plan-form";

/** `/plans/new`: a new plan, empty or started from somewhere else ("あそぼ!", a day of the calendar). */
export function PlanNewScreen() {
  const { t } = useTranslation("plans");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const friends = useFriends();
  const [draft] = useState<PlanDraft | null>(() => takePlanDraft());
  const [edited, setEdited] = useState<PlanFormValue | null>(null);
  const [tried, setTried] = useState(false);

  // Until the person types, the form follows the draft (its invitees appear once the friends are known).
  const friendUsers = (friends.data ?? []).flatMap((friend) => (friend.user ? [friend.user] : []));
  const value = edited ?? (draft ? formFromDraft(draft, friendUsers) : EMPTY_FORM);
  const problem = formProblem(value);

  const create = useMutation({
    mutationFn: () => planClient().createPlan(toCreateRequest(value)),
    onSuccess: ({ plan }) => {
      rememberPlan(queryClient, plan);
      toast.success(t("plan.created"));
      if (plan) void navigate({ to: "/plans/$planId", params: { planId: plan.id }, replace: true });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  return (
    <Page title={t("plan.newTitle")} back="/plans">
      <form
        className="flex flex-col gap-6"
        onSubmit={(event) => {
          event.preventDefault();
          setTried(true);
          if (!problem) create.mutate();
        }}
      >
        <PlanForm
          value={value}
          onChange={setEdited}
          withInvitees
          problem={tried && problem ? t(`plan.form.errors.${problem}`) : null}
        />
        <Button type="submit" size="lg" block loading={create.isPending}>
          {t("plan.create")}
        </Button>
      </form>
    </Page>
  );
}
