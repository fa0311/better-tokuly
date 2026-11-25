import { create, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { relationshipOf } from "./index";

describe("relationshipOf", () => {
  it("tells the viewer's own profile and a friend's apart", () => {
    const of = (relationship: identity.Relationship) =>
      relationshipOf(create(identity.ProfileSchema, { relationship }));
    expect(of(identity.Relationship.SELF)).toEqual({ isSelf: true, isFriend: false });
    expect(of(identity.Relationship.FRIENDS)).toEqual({ isSelf: false, isFriend: true });
    expect(relationshipOf(undefined)).toEqual({ isSelf: false, isFriend: false });
  });
});
