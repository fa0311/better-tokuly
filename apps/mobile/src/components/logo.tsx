import Svg, { Circle, Path } from "react-native-svg";

/**
 * The Tokuly mark (the web's `LogoMark`): a map pin with a round "now" dot
 * on a lime disc — a nod to the original logo. Its colours are the brand's
 * in both themes.
 */
export function LogoMark({ size = 40 }: { size?: number }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 64 64" accessibilityLabel="Tokuly">
      <Circle cx="32" cy="34" r="26" fill="#c6f24e" stroke="#17122b" strokeWidth="3" />
      <Path
        d="M32 8c-9.4 0-17 7.4-17 16.6 0 11.7 13.6 24.3 15.8 26.2a1.800 1.800 0 0 0 2.400 0C35.400 48.900 49 36.300 49 24.600 49 15.400 41.400 8 32 8Z"
        fill="#7c5cff"
        stroke="#17122b"
        strokeWidth="3"
        strokeLinejoin="round"
      />
      <Circle cx="32" cy="24.500" r="6.500" fill="#ffffff" stroke="#17122b" strokeWidth="3" />
      <Circle cx="46.500" cy="45.500" r="5" fill="#ff5c9d" stroke="#17122b" strokeWidth="3" />
    </Svg>
  );
}
