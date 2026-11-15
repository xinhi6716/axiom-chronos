#!/usr/bin/env python3
import argparse
import json
import os
from pathlib import Path
import random
import sys
import time


def main():
    parser = argparse.ArgumentParser()
    for name in ("dataset", "out", "base", "revision", "epochs", "seed", "device"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    if os.environ.get("FAKE_LAYA_TRAINER_LOG"):
        with open(os.environ["FAKE_LAYA_TRAINER_LOG"], "a", encoding="utf-8") as log:
            log.write(json.dumps(sys.argv[1:]) + "\n")
    if os.environ.get("FAKE_LAYA_TRAINER_FAIL") == "1":
        return 7
    deadline = time.monotonic() + float(os.environ.get("FAKE_LAYA_TRAINER_SLEEP", "0"))
    while time.monotonic() < deadline:
        time.sleep(min(0.05, max(0, deadline - time.monotonic())))
    out = Path(args.out)
    (out / "checkpoint").mkdir(parents=True, exist_ok=True)
    (out / "onnx").mkdir(parents=True, exist_ok=True)
    (out / "checkpoint" / "model.bin").write_bytes(b"fake checkpoint")
    (out / "onnx" / "model.onnx").write_bytes(b"fake onnx")
    (out / "tokenizer.json").write_text("{}", encoding="utf-8")
    labels = ["smoke", "manual", "none"]
    rng = random.Random(int(args.seed))
    with (out / "dev_scores.jsonl").open("w", encoding="utf-8") as scores:
        for i in range(12):
            gold = i % len(labels)
            logits = [rng.uniform(-0.5, 0.5) + (2.0 if j == gold else 0.0) for j in range(len(labels))]
            scores.write(json.dumps({"type": "needs", "label": labels[gold], "labels": labels, "logits": logits}) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
