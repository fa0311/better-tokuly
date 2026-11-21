#!/usr/bin/env python3
"""Turns the downloaded sources into the data files bundled with tokuly-geo.

Called by build.sh: build_data.py <work-dir> <output-dir>

Inputs (in <work-dir>):
  out/countries.ndjson, out/admin1.ndjson   simplified Natural Earth boundaries
  gn/cities500.zip, gn/JP.zip               GeoNames gazetteer extracts
  gn/alternatenames-JP.zip                  GeoNames alternate names for Japan
  gn/alternateNamesV2.zip                   GeoNames alternate names (optional: Japanese names abroad)
  gn/admin1CodesASCII.txt, gn/countryInfo.txt

Outputs (in <output-dir>):
  regions.ndjson.gz   {"id","kind","name","name_ja","cc","gn","geometry"} per line
  gazetteer.tsv.gz    lat, lng, cc, admin1, name, name_ja, population, area, area_ja
  admin1.tsv.gz       GeoNames admin1 key ("JP.40"), name, name_ja
  countries.tsv       ISO alpha-2, name, name_ja
"""
import gzip
import io
import json
import os
import re
import sys
import zipfile

WORK, OUT = sys.argv[1], sys.argv[2]

# Populated places smaller than this are left out abroad (Japan keeps every
# named place: they resolve the municipality).
MIN_POPULATION = 1000


def gz_write(name, text):
    path = os.path.join(OUT, name)
    # mtime=0 keeps the output byte-identical between runs.
    with open(path, "wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=9, mtime=0) as out:
        out.write(text.encode("utf-8"))
    print(f"  {name}: {os.path.getsize(path) / 1e6:.2f} MB")


def zip_lines(path, member):
    with zipfile.ZipFile(path) as archive, archive.open(member) as raw:
        for line in io.TextIOWrapper(raw, encoding="utf-8"):
            yield line.rstrip("\n").split("\t")


def clean(text):
    return text.replace("\t", " ").replace("\n", " ").strip()


# ── Regions ────────────────────────────────────────────────────────────────

def build_regions():
    countries, a3_to_id, lines = {}, {}, []
    for line in open(os.path.join(WORK, "out/countries.ndjson"), encoding="utf-8"):
        feature = json.loads(line)
        props = feature["properties"]
        if not feature.get("geometry"):
            continue
        countries[props["id"]] = props
        a3_to_id[props["a3"]] = props["id"]
        lines.append(json.dumps({
            "id": props["id"], "kind": "country", "name": props["name"], "name_ja": props.get("name_ja") or "",
            "cc": props["id"], "gn": "", "geometry": feature["geometry"],
        }, ensure_ascii=False, separators=(",", ":")))
    admin1_ja = {}
    count = 0
    for line in open(os.path.join(WORK, "out/admin1.ndjson"), encoding="utf-8"):
        feature = json.loads(line)
        props = feature["properties"]
        if not feature.get("geometry") or not props.get("id"):
            continue
        cc = props.get("cc") or a3_to_id.get(props.get("a3"), "")
        if cc not in countries:
            cc = a3_to_id.get(props.get("a3"), "")
        if not cc:
            continue  # disputed slivers without a country in the set
        name = props.get("name") or props["id"]
        if props.get("gn") and props.get("name_ja"):
            admin1_ja[props["gn"]] = props["name_ja"]
        lines.append(json.dumps({
            "id": props["id"], "kind": "admin1", "name": name, "name_ja": props.get("name_ja") or "",
            "cc": cc, "gn": props.get("gn") or "", "geometry": feature["geometry"],
        }, ensure_ascii=False, separators=(",", ":")))
        count += 1
    print(f"  regions: {len(countries)} countries, {count} first-level subdivisions")
    gz_write("regions.ndjson.gz", "\n".join(lines) + "\n")
    return countries, admin1_ja


# ── Japanese names ─────────────────────────────────────────────────────────

def pick_japanese(candidates):
    """Best of several `ja` alternate names: preferred, current, written in kanji/kana."""
    def rank(candidate):
        name, preferred, short, colloquial, historic = candidate
        has_kanji = any("一" <= ch <= "鿿" for ch in name)
        latin = any(ch.isascii() and ch.isalpha() for ch in name)
        return (historic == "1", colloquial == "1", latin, preferred != "1", not has_kanji, short == "1", len(name))
    return min(candidates, key=rank)[0] if candidates else ""


def japanese_names(wanted):
    """geonameid -> Japanese name, for the IDs in `wanted`."""
    found = {}
    sources = [("gn/alternatenames-JP.zip", "JP.txt"), ("gn/alternateNamesV2.zip", "alternateNamesV2.txt")]
    for path, member in sources:
        path = os.path.join(WORK, path)
        if not os.path.exists(path):
            print(f"  (no {os.path.basename(path)}: Japanese names abroad will be missing)")
            continue
        for cols in zip_lines(path, member):
            if len(cols) < 4 or cols[2] != "ja" or cols[1] not in wanted:
                continue
            flags = (cols[4:8] + [""] * 4)[:4]
            found.setdefault(cols[1], set()).add((cols[3], *flags))
    return {gid: clean(pick_japanese(sorted(names))) for gid, names in found.items()}


# ── Gazetteer ──────────────────────────────────────────────────────────────

SUFFIX = re.compile(r"[\s-](shi|ku|gun|machi|cho|mura|son|to|fu|ken)$", re.IGNORECASE)


def english_area(ascii_name):
    return SUFFIX.sub("", ascii_name).strip()


def build_gazetteer(countries_ne, admin1_ja_ne):
    # Japan: every named populated place, plus the municipalities they belong to.
    japan = list(zip_lines(os.path.join(WORK, "gn/JP.zip"), "JP.txt"))
    municipality = {c[0]: c for c in japan if c[7] == "ADM2"}        # by geonameid
    ward_or_town = {c[12]: c for c in japan if c[7] == "ADM3" and c[12]}  # by JIS code
    places = {}
    for c in japan:
        if c[6] == "P" and c[7] not in ("PPLH", "PPLQ", "PPLW", "PPLCH"):
            places[c[0]] = c
    world = 0
    for c in zip_lines(os.path.join(WORK, "gn/cities500.zip"), "cities500.txt"):
        if c[8] == "JP":
            places.setdefault(c[0], c)
            continue
        # Sections of a city (PPLX) would shadow the city itself.
        if c[7] in ("PPLX", "PPLH", "PPLQ", "PPLW", "PPLCH"):
            continue
        if int(c[14] or 0) >= MIN_POPULATION or c[7].startswith("PPLA") or c[7] == "PPLC":
            places[c[0]] = c
            world += 1

    admin1 = {}
    for line in open(os.path.join(WORK, "gn/admin1CodesASCII.txt"), encoding="utf-8"):
        key, name, _ascii, gid = line.rstrip("\n").split("\t")
        admin1[key] = (name, gid)
    countries = {}
    for line in open(os.path.join(WORK, "gn/countryInfo.txt"), encoding="utf-8"):
        if line.startswith("#"):
            continue
        cols = line.rstrip("\n").split("\t")
        countries[cols[0]] = (cols[4], cols[16])

    wanted = set(places) | set(municipality) | {c[0] for c in ward_or_town.values()}
    wanted |= {gid for _, gid in admin1.values()} | {gid for _, gid in countries.values()}
    ja = japanese_names(wanted)

    rows = []
    for gid, c in places.items():
        area = area_ja = ""
        if c[8] == "JP":
            city, part = municipality.get(c[11]), ward_or_town.get(c[12])
            if city:
                city_ja, city_en = ja.get(city[0], ""), english_area(city[2])
                area, area_ja = city_en, city_ja
                if part:
                    part_ja, part_en = ja.get(part[0], ""), english_area(part[2])
                    if city_ja.endswith(("郡", "支庁", "振興局")) or not city_ja:
                        # A town or village inside a district: the town is the locality.
                        area, area_ja = part_en, part_ja
                    elif part_ja:
                        # A ward of a designated city.
                        area, area_ja = f"{part_en}, {city_en}", city_ja + part_ja
        name_ja = ja.get(gid, "")
        rows.append((
            f"{float(c[4]):.4f}", f"{float(c[5]):.4f}", c[8], c[10], clean(c[1]),
            "" if name_ja == c[1] else name_ja, c[14] or "0", clean(area), clean(area_ja),
        ))
    rows.sort(key=lambda r: (r[2], r[3], r[4], r[0], r[1]))
    print(f"  gazetteer: {len(rows)} places ({len(rows) - world} in Japan), "
          f"{sum(1 for r in rows if r[5])} with a Japanese name, {sum(1 for r in rows if r[8])} with a municipality")
    gz_write("gazetteer.tsv.gz", "".join("\t".join(r) + "\n" for r in rows))

    lines = []
    for key in sorted(admin1):
        name, gid = admin1[key]
        lines.append(f"{key}\t{clean(name)}\t{ja.get(gid) or admin1_ja_ne.get(key, '')}\n")
    gz_write("admin1.tsv.gz", "".join(lines))

    lines = []
    for cc in sorted(countries):
        name, gid = countries[cc]
        # Natural Earth has the everyday name (フランス), GeoNames often the formal one.
        name_ja = (countries_ne.get(cc) or {}).get("name_ja") or ja.get(gid, "")
        lines.append(f"{cc}\t{clean(name)}\t{name_ja}\n")
    with open(os.path.join(OUT, "countries.tsv"), "w", encoding="utf-8") as out:
        out.writelines(lines)
    print(f"  countries.tsv: {len(lines)} countries, admin1.tsv.gz: {len(admin1)} subdivisions")


if __name__ == "__main__":
    ne_countries, ne_admin1_ja = build_regions()
    build_gazetteer(ne_countries, ne_admin1_ja)
