import { useInfiniteQuery } from "@tanstack/react-query";
import { admin } from "@tokuly/api";
import { moderationClient } from "../api";
import type { QueueSearch } from "./queue-search";

const SORTS = { oldest: admin.CaseSort.OLDEST, reports: admin.CaseSort.MOST_REPORTED } as const;

/** Pages through the cases matching `search`. The queue table and the rail beside a case share it. */
export function useCaseQueue(search: QueueSearch) {
  return useInfiniteQuery({
    queryKey: ["admin", "cases", search],
    queryFn: ({ pageParam }) =>
      moderationClient().listCases({
        page: { cursor: pageParam, limit: 30 },
        status: search.status === "resolved" ? admin.CaseStatus.RESOLVED : admin.CaseStatus.OPEN,
        assignee:
          search.assignee === "mine"
            ? admin.AssigneeFilter.MINE
            : search.assignee === "unassigned"
              ? admin.AssigneeFilter.UNASSIGNED
              : admin.AssigneeFilter.UNSPECIFIED,
        assigneeId:
          search.assignee && search.assignee !== "mine" && search.assignee !== "unassigned"
            ? search.assignee
            : "",
        subjectType: search.type,
        category: search.category,
        ownerId: search.owner,
        sort: search.sort ? SORTS[search.sort] : admin.CaseSort.UNSPECIFIED,
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}
