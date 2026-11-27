import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react-native";
import { community, create } from "@tokuly/api";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { MembersScreen } from "../members-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/store", () => ({ useMe: () => ({ roles: [] }) }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));

/** A public community, and whether its member list was asked for (the server refuses non-members). */
function server(status: community.MembershipStatus) {
  const asked: string[] = [];
  jest.mocked(api).mockReturnValue({
    service: () => ({
      getCommunity: async () => ({
        community: create(community.CommunitySchema, {
          id: "c1",
          handle: "neko",
          name: "ねこ部",
          visibility: community.CommunityVisibility.PUBLIC,
          viewer: { status },
        }),
      }),
      listMembers: async () => {
        asked.push("c1");
        return {
          members: [
            create(community.MemberSchema, {
              user: { id: "u1", handle: "aoi", displayName: "あおい" },
              role: community.MemberRole.MEMBER,
            }),
          ],
        };
      },
    }),
  } as never);
  return asked;
}

async function show() {
  await render(
    <QueryClientProvider client={testQueryClient()}>
      <MembersScreen handle="neko" />
    </QueryClientProvider>,
  );
}

describe("MembersScreen", () => {
  it("lists the members to a member", async () => {
    server(community.MembershipStatus.MEMBER);
    await show();
    expect(await screen.findByText("あおい")).toBeOnTheScreen();
  });

  it("tells someone outside that the list is the members', without asking for it", async () => {
    const asked = server(community.MembershipStatus.NONE);
    await show();
    expect(await screen.findByTestId("members-members-only")).toBeOnTheScreen();
    expect(asked).toEqual([]);
  });
});
