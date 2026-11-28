import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/react-query";
import { geo } from "@tokuly/api";
import { mapKeys } from "@tokuly/core/map";
import { journal } from "@tokuly/core/query";
import { describe, expect, it } from "vitest";
import { applyGeoEvent } from "./data";

/** A read of `key` with the journal, as the map's queries read; the test hands out its answer. */
function reading<T>(queryClient: QueryClient, key: readonly unknown[]) {
  let answer: (value: T) => void = () => {};
  const loading = queryClient.fetchQuery({
    queryKey: key,
    queryFn: () => new Promise<T>((resolve) => (answer = resolve)),
    persister: journal,
  });
  return { answer: (value: T) => answer(value), loading };
}

const push = (queryClient: QueryClient, event: geo.GeoEvent["event"]) =>
  applyGeoEvent(queryClient, create(geo.GeoEventSchema, { event }), "me");

// Where friends are: what a read made before a push must not bring back.
const gathering = create(geo.GatheringSchema, {
  id: "g1",
  center: { lat: 35.6595, lng: 139.7005 },
  members: [{ user: { id: "a" } }, { user: { id: "b" } }, { user: { id: "c" } }],
});

describe("gatherings and trips pushed during a read", () => {
  it("an ended gathering stays gone when an older read lands", async () => {
    const queryClient = new QueryClient();
    const read = reading<geo.Gathering[]>(queryClient, mapKeys.gatherings);
    push(queryClient, { case: "gatheringEndedId", value: "g1" });
    // The answer was made before the gathering ended.
    read.answer([gathering]);
    await read.loading;
    expect(queryClient.getQueryData(mapKeys.gatherings)).toEqual([]);
  });

  it("a member who left is not back in the gathering when an older read lands", async () => {
    const queryClient = new QueryClient();
    const read = reading<geo.Gathering[]>(queryClient, mapKeys.gatherings);
    const withoutC = { ...gathering, members: gathering.members.slice(0, 2) };
    push(queryClient, { case: "gatheringUpdated", value: withoutC });
    read.answer([gathering]);
    await read.loading;
    const [held] = queryClient.getQueryData<geo.Gathering[]>(mapKeys.gatherings) ?? [];
    expect(held?.members.map((member) => member.user?.id)).toEqual(["a", "b"]);
  });

  it("a trip that ended stays off the list when an older read lands", async () => {
    const queryClient = new QueryClient();
    const read = reading<geo.Trip[]>(queryClient, mapKeys.trips);
    const trip = create(geo.TripSchema, {
      id: "t1",
      user: { id: "a" },
      status: geo.TripStatus.ACTIVE,
    });
    push(queryClient, {
      case: "tripUpdated",
      value: { ...trip, status: geo.TripStatus.CANCELLED },
    });
    read.answer([trip]);
    await read.loading;
    expect(queryClient.getQueryData(mapKeys.trips)).toEqual([]);
  });
});
