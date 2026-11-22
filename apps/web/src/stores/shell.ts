import { useEffect } from "react";
import { create } from "zustand";

/** What pages ask of the frame around them. */
const useShell = create<{ tabBarHidden: number }>(() => ({ tabBarHidden: 0 }));

/**
 * Takes the phone tab bar away while the calling component is on screen: for
 * a page that needs the whole height (a conversation with its composer at
 * the bottom, a full-screen player). The rail of wide screens stays.
 *
 *   useHideTabBar();            // for as long as this screen is mounted
 *   useHideTabBar(playing);     // only while something is true
 */
export function useHideTabBar(hidden = true): void {
  useEffect(() => {
    if (!hidden) return;
    useShell.setState((state) => ({ tabBarHidden: state.tabBarHidden + 1 }));
    return () => useShell.setState((state) => ({ tabBarHidden: state.tabBarHidden - 1 }));
  }, [hidden]);
}

/** Whether some page on screen asked for the tab bar to be gone. */
export function useTabBarHidden(): boolean {
  return useShell((state) => state.tabBarHidden > 0);
}
