#!/usr/bin/env python3
"""Fails when a domain crate reaches into a domain it does not depend on.

The domain table in backend/README.md says which crates each domain may
depend on (besides core, proto and identity, which every crate may use).
This checks two things against it:

- every `tokuly-*` crate in a domain's [dependencies] is allowed;
- no SQL in a domain's source names a table created by the migrations of a
  domain it may not depend on.

    tools/ci/domain-boundaries.py
"""

import re
import sys
import tomllib
from pathlib import Path

BACKEND = Path(__file__).resolve().parents[2] / "backend"
ALWAYS = {"core", "proto", "identity"}

# Words that put a table name right after them in SQL (upper case, as all of ours is,
# so that prose like "from channels" in a comment does not count).
TABLE_CONTEXT = r"(?:FROM|JOIN|INTO|UPDATE|TABLE|REFERENCES)\s+(?:ONLY\s+)?"


def domain_table():
    """{domain: allowed dependencies}, from the README's "depends on" table."""
    allowed = {}
    for line in (BACKEND / "README.md").read_text().splitlines():
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if len(cells) == 3 and re.fullmatch(r"[a-z]+", cells[0]) and cells[0] != "crate":
            deps = {d.strip() for d in cells[2].split(",")} - {"—"}
            allowed[cells[0]] = deps | ALWAYS | {cells[0]}
    return allowed


def table_owners():
    """{table: domain}, from the CREATE statements of each domain's migrations."""
    owners = {}
    create = re.compile(r"CREATE\s+(?:UNLOGGED\s+)?(?:MATERIALIZED\s+)?(?:TABLE|VIEW)\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_][a-z0-9_]*)", re.I)
    for migration in sorted((BACKEND / "migrations").glob("[0-9]*_*.sql")):
        domain = migration.stem.split("_")[1]
        for table in create.findall(migration.read_text()):
            owners.setdefault(table.lower(), domain)
    return owners


def main():
    allowed = domain_table()
    owners = table_owners()
    problems = []

    for domain, may_use in sorted(allowed.items()):
        crate = BACKEND / "crates" / domain
        manifest = tomllib.loads((crate / "Cargo.toml").read_text())
        for dep in manifest.get("dependencies", {}):
            if dep.startswith("tokuly-") and dep.removeprefix("tokuly-") not in may_use:
                problems.append(f"crates/{domain}/Cargo.toml: depends on {dep}, which backend/README.md does not list for {domain}")

        foreign = {t: d for t, d in owners.items() if d not in may_use}
        if not foreign:
            continue
        pattern = re.compile(TABLE_CONTEXT + r"(" + "|".join(map(re.escape, foreign)) + r")\b")
        for source in sorted((crate / "src").rglob("*.rs")):
            for number, line in enumerate(source.read_text().splitlines(), 1):
                if line.lstrip().startswith("//"):
                    continue
                for table in pattern.findall(line):
                    problems.append(f"{source.relative_to(BACKEND)}:{number}: SQL on {table}, a table of {foreign[table]}")

    for problem in problems:
        print(problem)
    if problems:
        print(f"\n{len(problems)} reach(es) across domain boundaries (backend/README.md, the domain table)", file=sys.stderr)
        return 1
    print(f"ok: {len(allowed)} domains, {len(owners)} tables")
    return 0


if __name__ == "__main__":
    sys.exit(main())
