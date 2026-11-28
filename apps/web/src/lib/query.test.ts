import { journal } from "@tokuly/core/query";
import { describe, expect, it } from "vitest";
import { createQueryClient } from "./query";

describe("the app's query client", () => {
  it("lets a journaled query wait for the network like any other", () => {
    const options = createQueryClient().defaultQueryOptions({
      queryKey: ["journaled"],
      queryFn: () => 1,
      persister: journal,
    });
    expect(options.networkMode).toBe("online");
  });
});
