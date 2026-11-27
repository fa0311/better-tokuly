import { admin } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { ACCOUNT_STATES, ACTION_KINDS, REPORT_CATEGORIES } from "./filter-values";

/** Every value of a generated enum but `UNSPECIFIED` (0), in the enum's order. */
const valuesOf = (names: Record<string, string | number>) =>
  Object.values(names).filter((value) => typeof value === "number" && value !== 0);

describe("the console's filter values", () => {
  it("are every value of the API's enums", () => {
    expect(REPORT_CATEGORIES).toEqual(valuesOf(admin.ReportCategory));
    expect(ACTION_KINDS).toEqual(valuesOf(admin.ModerationActionKind));
    expect(ACCOUNT_STATES).toEqual(valuesOf(admin.AccountState));
  });
});
