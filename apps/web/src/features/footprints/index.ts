/**
 * What the footprints feature shares with other features:
 *
 *   celebrate(["🎉", "✨"])            // glyphs raining down the screen, for a moment worth it
 *   formatPercent(0.0244)              // "0.024%": explored shares, readable however small
 *   <FootprintMap userId={id} center={{ lat, lng }} zoom={12} />   // the scratch map of a person (empty id = me)
 */
export { celebrate } from "./celebrate";
export { FootprintMap } from "./footprint-map";
export { formatPercent } from "./levels";
