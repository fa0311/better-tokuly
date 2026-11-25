import { Redirect } from "expo-router";

/** A link the app has no screen for starts the app as usual instead of a dead end. */
export default function NotFound() {
  return <Redirect href="/" />;
}
