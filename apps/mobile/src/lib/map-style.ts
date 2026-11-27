import { buildMapStyle } from "@tokuly/core/map";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { TILE_CONFIG } from "#/lib/config";
import { useTheme } from "#/lib/theme";

/**
 * The basemap for MapLibre Native: the same as the web's, without the globe
 * and the sky (GL JS only). It is long (150 KB as JSON), so it is built and
 * serialized once per theme and language, not on every render of a screen
 * that renders again with every pin and every answer that comes in.
 */
export function useMapStyle(): string {
  const { dark } = useTheme();
  const { i18n } = useTranslation();
  return useMemo(() => {
    const {
      sky: _sky,
      projection: _projection,
      ...style
    } = buildMapStyle("tokuly", dark, i18n.language, TILE_CONFIG);
    return JSON.stringify(style);
  }, [dark, i18n.language]);
}
