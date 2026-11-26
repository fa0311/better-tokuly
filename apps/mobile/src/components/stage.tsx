import type { ReactNode } from "react";
import { View } from "react-native";
import { DARK_THEME, StageTheme } from "#/lib/theme";

/**
 * A black stage for video (the shorts feed): everything on it, the empty
 * state and its button included, takes the dark theme, so white controls
 * never end up on the light paper.
 */
export function DarkStage({ children, testID }: { children: ReactNode; testID?: string }) {
  return (
    <StageTheme.Provider value={DARK_THEME}>
      <View testID={testID} className="flex-1 bg-black" style={DARK_THEME.variables}>
        {children}
      </View>
    </StageTheme.Provider>
  );
}
