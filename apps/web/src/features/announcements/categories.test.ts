import { admin } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { ANNOUNCEMENT_CATEGORIES } from "./categories";

describe("ANNOUNCEMENT_CATEGORIES", () => {
  it("are the API's categories, in the filter's order", () => {
    expect(ANNOUNCEMENT_CATEGORIES.map((category) => admin.AnnouncementCategory[category])).toEqual(
      ["UPDATE", "EVENT", "MAINTENANCE", "IMPORTANT"],
    );
  });
});
