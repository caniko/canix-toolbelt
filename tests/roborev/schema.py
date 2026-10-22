"""Compare module field coverage with the exact packaged Roborev/Kit sources."""

import json
import re
import sys
from pathlib import Path


def structs(paths):
    result = {}
    for path in paths:
        text = path.read_text()
        for name, body in re.findall(r"^type (\w+) struct \{(.*?)^\}", text, re.MULTILINE | re.DOTALL):
            fields = {tag: typ for typ, tag in re.findall(r'^\s+\w+\s+(\S+)\s+`toml:"([^",]+)', body, re.MULTILINE)}
            embedded = re.findall(r"^\s+([A-Z]\w+)\s*$", body, re.MULTILINE)
            if fields:
                result[name] = (fields, embedded)
    return result


def check(source, vendor, projection):
    declared = json.loads(projection.read_text())
    definitions = structs(list((source / "internal/config").glob("*.go")))
    candidates = list(vendor.rglob("embedconfig/embedder.go"))
    assert len(candidates) == 1, "missing or ambiguous pinned Kit embedding source"
    definitions.update(structs(candidates))
    definitions.update(structs(list(vendor.rglob("secretref/secretref.go"))))
    for name, actual in declared.items():
        fields, embedded = definitions[name]
        fields = dict(fields)
        for parent in embedded:
            fields.update(definitions[parent][0])
        expected = set(fields)
        assert set(actual) == expected, (
            name,
            {"missing": sorted(expected - set(actual)), "extra": sorted(set(actual) - expected)},
        )
    # Enumerate paths recursively, retaining dynamic keys and list members.
    aliases = {"ACPAgentConfigs": "map[string]ACPAgentConfig", "BudgetAgentCosts": "map[string]int"}

    def paths(name, prefix):
        fields, embedded = definitions[name]
        fields = dict(fields)
        for parent in embedded:
            fields.update(definitions[parent][0])
        for field, typ in sorted(fields.items()):
            path = f"{prefix}.{field}"
            typ = aliases.get(typ, typ).lstrip("*")
            suffix = ""
            if typ.startswith("map[string]"):
                typ, suffix = typ[len("map[string]"):], ".*"
            elif typ.startswith("[]"):
                typ, suffix = typ[2:], "[]"
            typ = typ.removeprefix("embedconfig.")
            typ = typ.removeprefix("secretref.")
            if typ in declared:
                yield from paths(typ, path + suffix)
            else:
                yield {"path": path + suffix, "goType": typ}

    inventory = {name: list(paths(name, name)) for name in ["Config", "RepoConfig"]}
    print(json.dumps(inventory, indent=2))


if __name__ == "__main__":
    check(*(Path(value) for value in sys.argv[1:]))
