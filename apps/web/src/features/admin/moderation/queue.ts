import { useInfiniteQuery } from "@tanstack/react-query";
import { admin } from "@tokuly/api";
import { moderationClient } from "../api";
import { REPORT_CATEGORIES, SUBJECT_TYPES } from "../labels";
import { parseChoice, parseEnum, parseText } from "../params";

/**
 * The queue's filters as they appear in the address bar, so a filtered queue
 * can be bookmarked and survives opening a case and coming back. Defaults
 * (open cases, newest first, everyone's) are left out of the URL.
 */
export interface QueueSearch {
  status?: "resolved";
  category?: admin.ReportCategory;
  /** `mine`, `unassigned`, or the ID of one moderator. */
  assignee?: string;
  type?: (typeof SUBJECT_TYPES)[number];
  /** Only cases about content of this account. */
  owner?: string;
  sort?: "oldest" | "reports";
}

export function parseQueueSearch(search: Record<string, unknown>): QueueSearch {
  return {
    status: parseChoice(search.status, ["resolved"]),
    category: parseEnum(search.category, REPORT_CATEGORIES),
    assignee: parseText(search.assignee),
    type: parseEnum(search.type, SUBJECT_TYPES),
    owner: parseText(search.owner),
    sort: parseChoice(search.sort, ["oldest", "reports"]),
  };
}

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
