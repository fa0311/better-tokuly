import { describe, expect, it } from "vitest";
import { enqueue } from "./model";

describe("enqueue", () => {
  it("keeps the reveal on screen and drops the oldest waiting ones past the limit", () => {
    let queue: number[] = [];
    for (let id = 1; id <= 5; id++) queue = enqueue(queue, id, 3);
    expect(queue).toEqual([1, 4, 5]);
  });

  it("just appends below the limit", () => {
    expect(enqueue([1, 2], 3, 3)).toEqual([1, 2, 3]);
  });
});
