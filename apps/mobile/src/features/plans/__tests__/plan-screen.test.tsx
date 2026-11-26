import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { create, fromDate, plans } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Linking } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { PlanNewScreen } from "../plan-new-screen";
import { PlanScreen } from "../plan-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), replace: jest.fn(), back: jest.fn() },
  Stack: { Screen: () => null },
}));
// The map is the device's.
jest.mock("@maplibre/maplibre-react-native", () => {
  const { View } = require("react-native");
  const Stub = ({ children }: { children?: ReactNode }) =>
    require("react").createElement(View, null, children);
  return { Map: Stub, Camera: () => null, Marker: Stub };
});

const ME = { id: "me", handle: "me", displayName: "わたし" };
const YUI = { id: "yui", handle: "yui", displayName: "ゆい" };

const PLAN = create(plans.PlanSchema, {
  id: "p1",
  title: "放課後カラオケ",
  emoji: "🎤",
  status: plans.PlanStatus.ACTIVE,
  // 18:00 on Christmas Eve in Tokyo.
  time: {
    start: { case: "startAt", value: fromDate(new Date("2026-12-24T09:00:00Z")) },
    timeZone: "Asia/Tokyo",
  },
  place: { name: "駅前カラオケ", address: "渋谷1-2-3", location: { lat: 35.66, lng: 139.7 } },
  remindDayBefore: true,
  remindMinutesBefore: 60,
  myRole: plans.PlanRole.GUEST,
  myRsvp: plans.Rsvp.INVITED,
  counts: { going: 1, invited: 1 },
  participants: [
    { user: YUI, role: plans.PlanRole.HOST, rsvp: plans.Rsvp.GOING, comment: "たのしみ！" },
    { user: ME, role: plans.PlanRole.GUEST, rsvp: plans.Rsvp.INVITED },
  ],
});

/** The API of the account in use: the plan, the friends, and every request it was sent. */
function signIn() {
  const sent: { method: string; request: unknown }[] = [];
  const service = {
    getPlan: async () => ({ plan: PLAN }),
    listFriends: async () => ({ friends: [{ user: YUI }] }),
    respondToPlan: async (request: { rsvp: plans.Rsvp }) => {
      sent.push({ method: "respondToPlan", request });
      return { plan: { ...PLAN, myRsvp: request.rsvp }, rsvp: request.rsvp };
    },
    createPlan: async (request: unknown) => {
      sent.push({ method: "createPlan", request });
      return { plan: create(plans.PlanSchema, { id: "p2", title: "new" }) };
    },
  };
  jest.mocked(api).mockReturnValue({ service: () => service } as never);
  useSessionStore.setState({
    state: {
      status: "signedIn",
      sessionId: "s1",
      me: { user: ME, timeZone: "Asia/Tokyo" },
    } as never,
  });
  return sent;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("PlanScreen", () => {
  it("shows when and where on the account's clock, and sends the answer as the account", async () => {
    const sent = signIn();
    await show(<PlanScreen planId="p1" />);
    expect(await screen.findByTestId("plan-time")).toHaveTextContent("12月24日(木) 18:00〜");
    expect(screen.getByTestId("plan-reminders")).toHaveTextContent(
      "前の日にお知らせ · 60分前にお知らせ",
    );
    expect(screen.getByText("「たのしみ！」")).toBeOnTheScreen();
    expect(screen.getByText("行く？ 返事してね！")).toBeOnTheScreen();

    await act(() => fireEvent.press(screen.getByTestId("plan-rsvp-going")));
    expect(sent).toEqual([
      {
        method: "respondToPlan",
        request: { planId: "p1", rsvp: plans.Rsvp.GOING, comment: "" },
      },
    ]);
    expect(await screen.findByText("あなたの返事")).toBeOnTheScreen();
  });

  it("opens the place in the map app", async () => {
    signIn();
    const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    await show(<PlanScreen planId="p1" />);
    await act(async () => fireEvent.press(await screen.findByTestId("plan-place")));
    expect(openURL).toHaveBeenCalledWith(
      expect.stringMatching(/^(geo:35\.66,139\.7\?q=|maps:\?ll=35\.66,139\.7&q=)/),
    );
  });
});

describe("PlanNewScreen", () => {
  it("starts with the friend it was opened for and creates the plan on the account's clock", async () => {
    const sent = signIn();
    await show(<PlanNewScreen inviteId="yui" />);
    expect(await screen.findByText("ゆい")).toBeOnTheScreen();

    // Nothing is sent before the plan has a name.
    await act(() => fireEvent.press(screen.getByTestId("plan-create")));
    expect(screen.getByText("なにをするか書いてね")).toBeOnTheScreen();
    expect(sent).toEqual([]);

    await act(() => fireEvent.changeText(screen.getByTestId("plan-form-title"), "映画"));
    await act(() => fireEvent.press(screen.getByTestId("plan-when-time")));
    await act(() => fireEvent.press(screen.getByTestId("plan-form-date")));
    await act(() => fireEvent.press(screen.getByTestId("plan-day-next")));
    await act(() => fireEvent.press(screen.getByTestId("plan-day-next")));
    await act(() => fireEvent.press(screen.getByTestId(/^plan-day-\d{4}-\d{2}-10$/)));
    await act(() => fireEvent.press(screen.getByTestId("plan-form-time")));
    await act(() => fireEvent.press(screen.getByTestId("plan-hour-19")));
    await act(() => fireEvent.press(screen.getByTestId("plan-minute-30")));
    await act(() => fireEvent.press(screen.getByTestId("plan-create")));

    expect(sent).toHaveLength(1);
    const request = sent[0]?.request as plans.CreatePlanRequest;
    expect(request.title).toBe("映画");
    expect(request.inviteeIds).toEqual(["yui"]);
    expect(request.time?.timeZone).toBe("Asia/Tokyo");
    // 19:30 in Tokyo is 10:30 UTC.
    const start = request.time?.start;
    expect(
      start?.case === "startAt" && new Date(Number(start.value.seconds) * 1000).toISOString(),
    ).toMatch(/T10:30:00\.000Z$/);
    expect(router.replace).toHaveBeenCalledWith("/plans/p2");
  });

  it("finds the friend from their profile among many friends", async () => {
    signIn();
    const service = jest.mocked(api)().service(plans.PlanService) as unknown as Record<
      string,
      unknown
    >;
    // Only the first hundred friends come without a search; ゆい is not among them.
    service.listFriends = async ({ query }: { query: string }) => ({
      friends: query === "yui" ? [{ user: YUI }] : [],
    });
    await show(<PlanNewScreen inviteId="yui" inviteHandle="yui" />);
    expect(await screen.findByText("ゆい")).toBeOnTheScreen();
  });

  it("ends a night out after midnight on the next day", async () => {
    const sent = signIn();
    await show(<PlanNewScreen />);
    await act(() => fireEvent.changeText(screen.getByTestId("plan-form-title"), "カウントダウン"));
    await act(() => fireEvent.press(screen.getByTestId("plan-when-time")));
    await act(() => fireEvent.press(screen.getByTestId("plan-form-date")));
    await act(() => fireEvent.press(screen.getByTestId("plan-day-next")));
    await act(() => fireEvent.press(screen.getByTestId(/^plan-day-\d{4}-\d{2}-10$/)));
    await act(() => fireEvent.press(screen.getByTestId("plan-form-time")));
    await act(() => fireEvent.press(screen.getByTestId("plan-hour-22")));
    await act(() => fireEvent.press(screen.getByTestId("plan-minute-00")));
    await act(() => fireEvent(screen.getByTestId("plan-form-has-end"), "valueChange", true));
    await act(() => fireEvent.press(screen.getByTestId("plan-form-end-time")));
    await act(() => fireEvent.press(screen.getByTestId("plan-hour-01")));
    await act(() => fireEvent.press(screen.getByTestId("plan-minute-30")));
    await act(() => fireEvent.press(screen.getByTestId("plan-create")));

    const time = (sent[0]?.request as plans.CreatePlanRequest | undefined)?.time;
    const at = (part: plans.PlanTime["start"] | plans.PlanTime["end"] | undefined) =>
      part?.case === "startAt" || part?.case === "endAt" ? Number(part.value.seconds) : 0;
    expect(at(time?.end) - at(time?.start)).toBe(3.5 * 3600);
  });
});
