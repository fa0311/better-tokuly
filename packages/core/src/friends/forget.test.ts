import { QueryClient, QueryObserver } from "@tanstack/query-core";
import { create, social } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { forgetUserFriends } from "./forget";
import { followKeys, friendKeys } from "./keys";

const friend = (id: string) => create(social.FriendSchema, { user: { id } });

const list = (...ids: string[]) =>
  create(social.ListFriendsResponseSchema, { friends: ids.map(friend), total: ids.length });

const ids = (data: social.ListFriendsResponse | undefined) =>
  data?.friends.map((one) => one.user?.id);

describe("forgetting a friend", () => {
  it("takes them out of every loaded friends list at once", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(friendKeys.list(""), list("aoi", "ren"));
    queryClient.setQueryData(friendKeys.list("a"), list("aoi"));
    void forgetUserFriends(queryClient, "aoi");
    expect(ids(queryClient.getQueryData(friendKeys.list("")))).toEqual(["ren"]);
    expect(queryClient.getQueryData<social.ListFriendsResponse>(friendKeys.list(""))?.total).toBe(
      1,
    );
    expect(ids(queryClient.getQueryData(friendKeys.list("a")))).toEqual([]);
  });

  it("drops what was loaded about their follows", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(followKeys.summary("aoi"), {});
    queryClient.setQueryData(followKeys.summary("ren"), {});
    void forgetUserFriends(queryClient, "aoi");
    expect(queryClient.getQueryData(followKeys.summary("aoi"))).toBeUndefined();
    expect(queryClient.getQueryData(followKeys.summary("ren"))).toBeDefined();
  });

  it("reads the requests again, and a read in flight does not count", async () => {
    const queryClient = new QueryClient();
    const answers: ((value: string) => void)[] = [];
    const stop = new QueryObserver(queryClient, {
      queryKey: friendKeys.incoming,
      queryFn: () => new Promise<string>((resolve) => answers.push(resolve)),
    }).subscribe(() => {});
    expect(answers).toHaveLength(1);
    const forgotten = forgetUserFriends(queryClient, "aoi");
    expect(answers).toHaveLength(2);
    answers[0]?.("with their request");
    answers[1]?.("without");
    await forgotten;
    expect(queryClient.getQueryData(friendKeys.incoming)).toBe("without");
    stop();
  });
});
