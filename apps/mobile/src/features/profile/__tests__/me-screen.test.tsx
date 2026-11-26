import { fireEvent, render, screen } from "@testing-library/react-native";
import { router } from "expo-router";
import Me from "../../../../app/(app)/(tabs)/me";

jest.mock("expo-router", () => {
  const React = require("react");
  const router = { push: jest.fn() };
  return {
    router,
    Link: ({ children, href }: { children: React.ReactElement; href: string }) =>
      React.cloneElement(children, { onPress: () => router.push(href) }),
  };
});
jest.mock("react-i18next", () => ({ useTranslation: () => ({}) }));
jest.mock("#/features/profile/profile-screen", () => ({
  ProfileScreen: ({ footer }: { footer: React.ReactNode }) => footer,
}));
jest.mock("#/features/session/store", () => ({ useMe: () => ({ user: { handle: "me" } }) }));
jest.mock("#/seams/badges", () => ({ useBadges: () => ({ friends: 2 }) }));
jest.mock("#/seams/hub", () => ({
  HUB_ENTRIES: [
    { id: "friends", href: "/friends", label: () => "ともだち", badge: "friends" },
    { id: "settings", href: "/settings", label: () => "設定" },
  ],
}));

test("the hub displays its links and badge, and opens settings", async () => {
  await render(<Me />);
  expect(screen.getByText("ともだち 2")).toBeOnTheScreen();
  expect(screen.getByText("設定")).toBeOnTheScreen();
  await fireEvent.press(screen.getByTestId("me-settings"));
  expect(router.push).toHaveBeenCalledWith("/settings");
});
