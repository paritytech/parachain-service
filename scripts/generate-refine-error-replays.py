#!/usr/bin/env python3
"""Generate normalized ITF fixtures with Quint 0.32.0 and the vendored model."""

import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "service/bin/tests/fixtures/quint"


def main():
    version = subprocess.check_output(["quint", "--version"], text=True).strip()
    if version != "0.32.0":
        raise SystemExit(f"Expected Quint 0.32.0, found {version}")
    with tempfile.TemporaryDirectory(prefix="refine-replays-") as output:
        subprocess.run(
            [
                "quint", "test", str((FIXTURES / "refine_errors.qnt").relative_to(ROOT)),
                "--backend", "typescript", "--seed", "1", "--match", "_works$",
                "--out-itf", str(Path(output) / "{test}.itf.json"),
            ],
            cwd=ROOT,
            check=True,
        )
        traces = sorted(Path(output).glob("*.itf.json"))
        if len(traces) != 10:
            raise SystemExit(f"Expected 10 traces, found {len(traces)}")
        destination = FIXTURES / "refine_errors"
        destination.mkdir(exist_ok=True)
        for path in traces:
            trace = json.loads(path.read_text())
            # Metadata has timestamps and bare JSON integers, which the Rust
            # ITF value decoder rejects. Preserve every generated state value.
            trace.pop("#meta", None)
            trace["vars"].sort()
            for state in trace["states"]:
                state.pop("#meta", None)
            (destination / path.name).write_text(
                json.dumps(trace, indent=2, sort_keys=True) + "\n"
            )


if __name__ == "__main__":
    main()
