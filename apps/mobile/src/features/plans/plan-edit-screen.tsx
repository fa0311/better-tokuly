import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { plans } from "@tokuly/api";
import {
  formFromPlan,
  formProblem,
  type PlanFormValue,
  rememberPlan,
  toUpdateRequest,
} from "@tokuly/core/plans";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { asViewer, showPlanError } from "./actions";
import { usePlan, usePlanZone } from "./data";
import { PlanForm } from "./plan-form";

/**
 * Changes a plan (its host and co-hosts). Only what changed is sent; a new
 * time or place tells everyone in the plan.
 */
export function PlanEditScreen({ planId }: { planId: string }) {
  const { t } = useTranslation("plans");
  const { data: plan, isPending } = usePlan(planId);
  if (isPending) return <Spinner />;
  if (!plan) return <EmptyState emoji="🫥" title={t("plan.notFound")} />;
  return <EditForm plan={plan} />;
}

function EditForm({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const zone = usePlanZone();
  const queryClient = useQueryClient();
  const [before] = useState(() => formFromPlan(plan, zone));
  const [value, setValue] = useState<PlanFormValue>(before);
  const problem = formProblem(value, zone);

  const save = useMutation({
    mutationFn: () =>
      asViewer((service) => service.updatePlan(toUpdateRequest(plan.id, before, value, zone))),
    onSuccess: (response) => {
      if (!response) return;
      rememberPlan(queryClient, response.plan);
      router.back();
    },
    onError: showPlanError,
  });

  return (
    <Page testID="plan-edit">
      <PlanForm
        value={value}
        onChange={setValue}
        problem={problem ? t(`plan.form.errors.${problem}`) : null}
      />
      <Button
        testID="plan-save"
        size="lg"
        block
        disabled={Boolean(problem)}
        loading={save.isPending}
        onPress={() => save.mutate()}
      >
        {t("plan.save")}
      </Button>
    </Page>
  );
}
