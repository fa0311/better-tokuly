import { fireEvent, render, screen } from "@testing-library/react-native";
import { StyleSheet } from "react-native";
import { Avatar } from "../avatar";
import { Button } from "../button";
import { Tabs } from "../tabs";

const START = "はじめる";

describe("Button", () => {
  // NativeWind drops a style function from a component with a className: the pill must be a plain style.
  it("is a pill with the sticker outline, as a plain style", async () => {
    await render(<Button testID="go">{START}</Button>);
    const button = screen.getByTestId("go");
    expect(typeof button.props.style).not.toBe("function");
    expect(StyleSheet.flatten(button.props.style)).toMatchObject({
      borderRadius: 999,
      borderWidth: 2,
      height: 44,
      backgroundColor: "#7859f7",
    });
  });

  it("does nothing while it is loading", async () => {
    const onPress = jest.fn();
    await render(
      <Button testID="go" loading onPress={onPress}>
        {START}
      </Button>,
    );
    await fireEvent.press(screen.getByTestId("go"));
    expect(onPress).not.toHaveBeenCalled();
    expect(screen.queryByText(START)).toBeNull();
  });
});

describe("Avatar", () => {
  it("leaves the tile's colour to its class when there is no picture", async () => {
    await render(<Avatar user={{ id: "u1", displayName: "Aoi", handle: "aoi", deleted: false }} />);
    const tile = screen.getByText("A").parent?.parent;
    expect(StyleSheet.flatten(tile?.props.style)).not.toHaveProperty("backgroundColor");
  });
});

describe("Tabs", () => {
  it("marks the chosen view and reports another one tapped", async () => {
    const onChange = jest.fn();
    await render(
      <Tabs
        testID="tab"
        items={[
          { value: "inbox", label: "トーク", badge: 3 },
          { value: "requests", label: "リクエスト" },
        ]}
        value={"inbox" as "inbox" | "requests"}
        onChange={onChange}
      />,
    );
    expect(screen.getByTestId("tab-inbox").props.accessibilityState).toEqual({ selected: true });
    expect(screen.getByText("3")).toBeTruthy();
    await fireEvent.press(screen.getByTestId("tab-requests"));
    expect(onChange).toHaveBeenCalledWith("requests");
  });
});
