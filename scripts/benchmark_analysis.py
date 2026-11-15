#!/usr/bin/env python3
"""Paired full/adaptive runs, alternating order. No quality claim without labels."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess
import time


def benchmark(binary, common, out, repeats=3):
    if repeats < 1:
        raise ValueError("repeats must be positive")
    out = Path(out)
    out.mkdir(parents=True, exist_ok=False)
    samples = {"full": [], "adaptive": []}
    for repeat in range(repeats):
        order = ("full", "adaptive") if repeat % 2 == 0 else ("adaptive", "full")
        for mode in order:
            stem = out / f"{mode}-{repeat + 1}"
            command = [str(binary), "run", *common, "--json", str(stem.with_suffix(".jsonl")),
                       "--metrics", str(stem.with_suffix(".metrics.json"))]
            if mode == "adaptive":
                command.append("--adaptive")
            started = time.perf_counter()
            with stem.with_suffix(".log").open("w") as log:
                subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
            elapsed = time.perf_counter() - started
            metrics = json.loads(stem.with_suffix(".metrics.json").read_text())
            samples[mode].append({"process_seconds": elapsed, "metrics": metrics, "command": command})
    medians = {mode: statistics.median(r["process_seconds"] for r in runs) for mode, runs in samples.items()}
    # Same frame counts/time span are essential: dropping frames is not speedup.
    spans = {(r["metrics"]["frames"], r["metrics"]["source_start"], r["metrics"]["source_end"])
             for runs in samples.values() for r in runs}
    if len(spans) != 1:
        raise ValueError("runs do not cover identical frames")
    report = {"schema": 1, "median_seconds": medians,
              "speedup": medians["full"] / medians["adaptive"],
              "fivefold_time_target_met": medians["adaptive"] <= medians["full"] / 5,
              "quality_validated": False, "samples": samples}
    (out / "benchmark.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new result directory")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("run_args", nargs=argparse.REMAINDER, help="after --: video and normal run options")
    args = parser.parse_args()
    common = args.run_args[1:] if args.run_args[:1] == ["--"] else args.run_args
    forbidden = {"--adaptive", "--json", "--metrics", "--minimap", "--pip", "--alpha", "--dets", "--ticker-json"}
    if not common or any(a.split("=", 1)[0] in forbidden for a in common):
        parser.error("pass video/options, without output or adaptive flags (JSON-only comparison)")
    report = benchmark(args.binary.resolve(), common, args.out, args.repeats)
    print(json.dumps({k: v for k, v in report.items() if k != "samples"}, indent=2))


if __name__ == "__main__":
    main()
