/**
 * The attribution the gazetteer's licence asks for. Show it wherever names
 * of areas that came from the server's place-name data are on screen
 * (addresses, search hits, the area a visit happened in). Not translated:
 * it is a credit line.
 */
export function GazetteerCredit({ className }: { className?: string }) {
  return (
    <p className={`text-[11px] text-ink-faint ${className ?? ""}`}>
      Place names ©{" "}
      <a href="https://www.geonames.org/" target="_blank" rel="noreferrer" className="underline">
        GeoNames
      </a>
      ,{" "}
      <a
        href="https://creativecommons.org/licenses/by/4.0/"
        target="_blank"
        rel="noreferrer"
        className="underline"
      >
        CC BY 4.0
      </a>
    </p>
  );
}
