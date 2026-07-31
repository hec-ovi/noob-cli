#!/usr/bin/env python3
"""Check every fixture against the schema its name points at.

The crate's Rust tests prove the behaviour: every variant of both enums
round-trips, escaping survives the nasty strings, a future version is refused.
This proves the *boundary*: that what crosses the wire matches the declared
shape, and that a malformed frame is rejected rather than quietly accepted.

Every fixture under `valid/` whose name starts with `event` or `command` is a
line the real encoder produced, byte for byte minus the trailing newline.

A fixture is named after its schema. Anything after a double dash is a note
about the case, so `event--tool-end.json` and `event--note.json` both validate
against `schema/event.json`. Fixtures under `invalid/` must fail; if one starts
passing, the schema has been loosened and the boundary no longer fails closed.

Run: python3 tests/contract.py
"""

import json
import pathlib
import sys

try:
    from jsonschema import Draft202012Validator, RefResolver
except ImportError:
    sys.exit("contract.py needs jsonschema: pip install jsonschema")

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "schema"


def schema_for(fixture: pathlib.Path) -> pathlib.Path:
    return SCHEMA / f"{fixture.stem.split('--')[0]}.json"


def validator_for(path: pathlib.Path) -> Draft202012Validator:
    schema = json.loads(path.read_text())
    # The schemas cross-reference each other by bare filename, resolved against
    # this directory. Every one is preloaded into the store so a ref can never
    # reach the network, which a contract test must not depend on.
    store = {f"{SCHEMA.as_uri()}/{p.name}": json.loads(p.read_text()) for p in SCHEMA.glob("*.json")}
    resolver = RefResolver(base_uri=f"{SCHEMA.as_uri()}/", referrer=schema, store=store)
    return Draft202012Validator(schema, resolver=resolver)


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
        print(f"noob-proto contract: {len(failures)} problem(s)")
        for line in failures:
            print(f"  {line}")
        return 1

    print(f"noob-proto contract: {checked} fixtures pass, {len(list(SCHEMA.glob('*.json')))} schemas covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
