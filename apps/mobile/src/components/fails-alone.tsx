import { Component, type ReactNode } from "react";

/**
 * Something mounted for the whole session (a background feature, a badge's
 * count) that threw while rendering: it stops alone, for the rest of the
 * account's session, and the screens go on; the next account mounts it anew.
 * React Native logs the error, as for a screen (`screen-error.tsx`).
 */
export class FailsAlone extends Component<{ children: ReactNode }, { failed: boolean }> {
  override state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  override render() {
    return this.state.failed ? null : this.props.children;
  }
}
