import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { plans } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { journal } from "../query";
import { applyPlansEvent } from "./cache";
import { planKeys } from "./keys";
import { type PlanPage, planQueries } from "./queries";

const plan = create(plans.PlanSchema, { id: "p1", title: "Karaoke" });
const poll = create(plans.PollSchema, { id: "q1", shortId: "abc" });

/** Answers the test hands out, one per request, in the order the requests were made. */
function server<T>() {
  const pending: ((answer: T) => void)[] = [];
  return {
    request: () => new Promise<T>((resolve) => pending.push(resolve)),
    answer: (request: number, answer: T) => pending[request]?.(answer),
    get requests() {
      return pending.length;
    },
  };
}

const removed = (planId: string): plans.PlansEvent["event"] => ({
  case: "planChanged",
  value: create(plans.PlanChangedSchema, { planId, change: plans.PlanChange.REMOVED }),
});

describe("plans pushed during a read", () => {
  it("a plan the person was removed from stays gone when an older read lands", async () => {
    const queryClient = new QueryClient();
    const api = server<plans.GetPlanResponse>();
    const client = () => ({ getPlan: api.request }) as never;
    const loading = queryClient.fetchQuery(planQueries.plan(client, "p1"));
    applyPlansEvent(queryClient, removed("p1"));
    // The answer was made before the removal.
    api.answer(0, create(plans.GetPlanResponseSchema, { plan }));
    await loading;
    expect(queryClient.getQueryData(planKeys.plan("p1"))).toBeNull();
  });

  it("a checklist item pushed during a read is on the plan it reads", async () => {
    const queryClient = new QueryClient();
    const api = server<plans.GetPlanResponse>();
    const client = () => ({ getPlan: api.request }) as never;
    const loading = queryClient.fetchQuery(planQueries.plan(client, "p1"));
    const item = create(plans.ChecklistItemSchema, { id: "i1", text: "マイク" });
    applyPlansEvent(queryClient, {
      case: "checklistChanged",
      value: create(plans.ChecklistChangedSchema, { planId: "p1", item }),
    });
    api.answer(0, create(plans.GetPlanResponseSchema, { plan }));
    await loading;
    const read = queryClient.getQueryData<plans.Plan>(planKeys.plan("p1"));
    expect(read?.checklist.map((entry) => entry.id)).toEqual(["i1"]);
  });

  it("a list being read for the first time is read again after a removal, not joined", async () => {
    const queryClient = new QueryClient();
    const api = server<plans.ListPlansResponse>();
    const client = () => ({ listPlans: api.request }) as never;
    const options = planQueries.list(client, plans.PlanListFilter.UPCOMING);
    const loading = queryClient.fetchInfiniteQuery(options);
    applyPlansEvent(queryClient, removed("p1"));
    // The read from before the removal is dropped; whoever waited on it gets the next one.
    api.answer(0, create(plans.ListPlansResponseSchema, { plans: [plan] }));
    api.answer(1, create(plans.ListPlansResponseSchema, { plans: [] }));
    const list = await loading;
    expect(list.pages.flatMap((page: PlanPage) => page.plans)).toEqual([]);
  });

  it("a poll deleted while its link is first read stays gone", async () => {
    const queryClient = new QueryClient();
    const api = server<plans.Poll>();
    // Read by the short ID of the link: until it lands, nothing says which poll it is.
    const loading = queryClient.fetchQuery({
      queryKey: planKeys.poll("abc"),
      queryFn: api.request,
      persister: journal,
    });
    applyPlansEvent(queryClient, {
      case: "pollChanged",
      value: create(plans.PollChangedSchema, { pollId: "q1", deleted: true }),
    });
    api.answer(0, poll);
    await loading;
    expect(queryClient.getQueryData(planKeys.poll("abc"))).toBeNull();
  });

  it("another poll's read is left as it lands", async () => {
    const queryClient = new QueryClient();
    const api = server<plans.Poll>();
    const loading = queryClient.fetchQuery({
      queryKey: planKeys.poll("other"),
      queryFn: api.request,
      persister: journal,
    });
    applyPlansEvent(queryClient, {
      case: "pollChanged",
      value: create(plans.PollChangedSchema, { pollId: "q1", deleted: true }),
    });
    const other = create(plans.PollSchema, { id: "q2", shortId: "other" });
    api.answer(0, other);
    await loading;
    expect(queryClient.getQueryData(planKeys.poll("other"))).toEqual(other);
  });
});
