#!/usr/bin/env python3
"""Switch committed Quint replay fixtures between readable and compact JSON."""

import argparse
import json
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent.parent / "service/bin/tests/fixtures/quint"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["pretty", "compact"])
    args = parser.parse_args()
    formatting = {"indent": 2} if args.mode == "pretty" else {"separators": (",", ":")}
    for path in sorted(FIXTURES.rglob("*.itf.json")):
        value = json.loads(path.read_text())
        path.write_text(json.dumps(value, sort_keys=True, **formatting) + "\n")


if __name__ == "__main__":
    main()
