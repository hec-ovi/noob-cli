#!/usr/bin/env python3
"""Check every fixture against the schema its name points at.

The box's Rust tests prove the behaviour: items round-trip, repair heals a
torn tail, resume rebuilds the exact transcript. This proves the *boundary*:
that every line the writer appends matches its declared shape. The reader is
deliberately tolerant (a corrupt line is skipped and counted), so the schema
binds the writer; the Rust test `the_committed_fixtures_match_the_writer`
keeps the valid fixtures honest against the real encoder.

A fixture is named after its schema. Anything after a double dash is a note
about the case, so `item--user.json` and `item--tool.json` both validate
against `schema/item.json`. Fixtures under `invalid/` must fail; if one starts
passing, the schema has been loosened and the boundary no longer fails closed.

Run: python3 tests/contract.py
"""

import json
import pathlib
import sys

try:
    from jsonschema import Draft202012Validator
    from referencing import Registry, Resource
    from referencing.jsonschema import DRAFT202012
except ImportError:
    sys.exit("contract.py needs jsonschema: pip install jsonschema")

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "schema"


def schema_for(fixture: pathlib.Path) -> pathlib.Path:
    return SCHEMA / f"{fixture.stem.split('--')[0]}.json"


def validator_for(path: pathlib.Path) -> Draft202012Validator:
    schema = json.loads(path.read_text())
    # The schemas cross-reference each other by bare filename. Every one is
    # preloaded into the registry under that name so a ref can never reach the
    # network, which a contract test must not depend on.
    registry = Registry().with_resources(
        (p.name, Resource.from_contents(json.loads(p.read_text()), default_specification=DRAFT202012))
        for p in SCHEMA.glob("*.json")
    )
    return Draft202012Validator(schema, registry=registry)


def main() -> int:
    failures = []
    checked = 0

    for fixture in sorted((ROOT / "fixtures" / "valid").glob("*.json")):
        target = schema_for(fixture)
        if not target.exists():
            failures.append(f"{fixture.name}: no schema named {target.name}")
            continue
        errors = list(validator_for(target).iter_errors(json.loads(fixture.read_text())))
        checked += 1
        if errors:
            why = "; ".join(e.message for e in errors)
            failures.append(f"{fixture.name} should validate against {target.name}: {why}")

    for fixture in sorted((ROOT / "fixtures" / "invalid").glob("*.json")):
        target = schema_for(fixture)
        if not target.exists():
            failures.append(f"{fixture.name}: no schema named {target.name}")
            continue
        errors = list(validator_for(target).iter_errors(json.loads(fixture.read_text())))
        checked += 1
        if not errors:
            failures.append(
                f"{fixture.name} validated against {target.name} but must be rejected: "
                "the boundary stopped failing closed"
            )

    # Every declared schema needs at least one fixture, or a shape can be
    # published in the contract and never exercised.
    covered = {schema_for(f).name for f in (ROOT / "fixtures").rglob("*.json")}
    for schema in sorted(SCHEMA.glob("*.json")):
        if schema.name not in covered:
            failures.append(f"{schema.name} is declared but has no fixture")

    if failures:
        print(f"session contract: {len(failures)} problem(s)")
        for line in failures:
            print(f"  {line}")
        return 1

    print(f"session contract: {checked} fixtures pass, {len(list(SCHEMA.glob('*.json')))} schemas covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
