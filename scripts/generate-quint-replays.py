#!/usr/bin/env python3
"""Generate normalized ITF fixtures with Quint 0.32.0 and the vendored model."""

import json
import re
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "service/bin/tests/fixtures/quint"


def main():
    version = subprocess.check_output(["quint", "--version"], text=True).strip()
    if version != "0.32.0":
        raise SystemExit(f"Expected Quint 0.32.0, found {version}")
    for source in ["refine_errors", "blocks", "upgrades", "log_pruning", "kv", "balances"]:
        generate(source)


def generate(source):
    with tempfile.TemporaryDirectory(prefix="quint-replays-") as output:
        subprocess.run(
            [
                "quint", "test", str((FIXTURES / f"{source}.qnt").relative_to(ROOT)),
                "--backend", "typescript", "--seed", "1", "--match", "_works$",
                "--out-itf", str(Path(output) / "{test}.itf.json"),
            ],
            cwd=ROOT,
            check=True,
        )
        traces = sorted(Path(output).glob("*.itf.json"))
        expected = set(re.findall(
            r"^  run (\w+_works) =", (FIXTURES / f"{source}.qnt").read_text(), re.MULTILINE
        ))
        actual = {path.name.removesuffix(".itf.json") for path in traces}
        if actual != expected:
            raise SystemExit(f"Trace mismatch: missing={expected - actual}, extra={actual - expected}")
        destination = FIXTURES / source
        destination.mkdir(exist_ok=True)
        for path in traces:
            trace = json.loads(path.read_text())
            # Metadata has timestamps and bare JSON integers, which the Rust
            # ITF value decoder rejects. Preserve every generated state value.
            trace.pop("#meta", None)
            trace["vars"].sort()
            for state in trace["states"]:
                state.pop("#meta", None)
            root_fixtures = {
                "minimal_replay_works.itf.json": "minimal_replay.itf.json",
                "stale_parent_candidate_rejected_works.itf.json": "staleParentCandidateRejectedTest.itf.json",
            }
            target = (FIXTURES / root_fixtures[path.name]
                      if path.name in root_fixtures else destination / path.name)
            target.write_text(
                json.dumps(trace, separators=(",", ":"), sort_keys=True) + "\n"
            )


if __name__ == "__main__":
    main()
