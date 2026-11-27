import { QueryClient } from "@tanstack/query-core";
import { create, school } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { loadedAssignments, schoolQueries } from "./queries";

describe("schoolQueries.assignments", () => {
  it("reads past the first page with the cursor the server gave", async () => {
    const asked: string[] = [];
    const client = () =>
      ({
        listAssignments: async ({ page }: { page: { cursor: string } }) => {
          asked.push(page.cursor);
          const first = page.cursor === "";
          return create(school.ListAssignmentsResponseSchema, {
            assignments: [{ id: first ? "a1" : "a2" }],
            page: { nextCursor: first ? "c2" : "", hasMore: first },
            openCount: 2,
            overdueCount: 1,
          });
        },
      }) as never;
    const data = await new QueryClient().fetchInfiniteQuery({
      ...schoolQueries.assignments(client, school.AssignmentFilter.OPEN),
      pages: 3,
    });
    expect(asked).toEqual(["", "c2"]);
    const loaded = loadedAssignments(data.pages);
    expect(loaded?.assignments.map((a) => a.id)).toEqual(["a1", "a2"]);
    expect([loaded?.openCount, loaded?.overdueCount]).toEqual([2, 1]);
  });

  it("is nothing until the first page is there", () => {
    expect(loadedAssignments(undefined)).toBeUndefined();
  });
});
