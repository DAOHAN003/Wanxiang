#!/usr/bin/env python3
"""Check that the published checkpoint schema advertises the SDK envelope contract."""

import json
from pathlib import Path


def main() -> None:
    schema_path = Path(__file__).parents[1] / "schemas" / "wanxiang-checkpoint-v1.schema.json"
    schema = json.loads(schema_path.read_text(encoding="utf-8"))

    assert schema.get("$schema") == "https://json-schema.org/draft/2020-12/schema"
    assert schema.get("type") == "object"
    assert set(schema.get("required", [])) == {"format", "schema_version", "payload"}
    assert schema["properties"]["format"].get("const") == "wanxiang-checkpoint"
    assert schema["properties"]["schema_version"].get("const") == 1
    assert schema.get("additionalProperties") is False

    payload = schema["properties"]["payload"]
    assert payload.get("type") == "object"
    assert set(payload.get("required", [])) == {
        "seed", "rules", "tick", "initialization", "operations"
    }
    assert payload.get("additionalProperties") is False

    init_schema = payload["properties"]["initialization"]
    assert {"const": "Seeded"} in init_schema.get("oneOf", [])
    assert any(option.get("type") == "object" for option in init_schema["oneOf"])

    operations = payload["properties"]["operations"]
    assert operations.get("type") == "array"
    operation_item = operations["items"]
    assert operation_item.get("type") == "object"
    assert operation_item.get("minProperties") == operation_item.get("maxProperties") == 1

    print("Validated versioned checkpoint v1 schema metadata and operation-envelope shape.")


if __name__ == "__main__":
    main()
