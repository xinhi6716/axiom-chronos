#!/usr/bin/env python3
"""Train/export Laya and fit the runtime calibration artifact."""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys


TRAIN = r'''
import sys
sys.path.insert(0, sys.argv[1])
from huggingface_hub import snapshot_download
import laya.train as train
from laya.train_cli import main
upstream, base, revision = sys.argv[1:4]
checkpoint = snapshot_download(repo_id=base, revision=revision)
train.split_calibration = lambda items, *args: (list(items), [])
raise SystemExit(main(["--data", sys.argv[4], "--out", sys.argv[5], "--base", checkpoint,
                       "--epochs", sys.argv[6], "--seed", sys.argv[7], "--device", sys.argv[8]]))
'''

SCORE = r'''
import json, sys
sys.path.insert(0, sys.argv[1])
from laya.train import load_checkpoint, items_from_rows, calibration_records, resolve_device, uses_parallel_layout
checkpoint, dev_path, dest, device_name = sys.argv[2:6]
model, tok, cfg = load_checkpoint(checkpoint)
device = resolve_device(device_name)
model.to(device)
max_len, head_max_len = cfg.get("max_len", 512), cfg.get("head_max_len", 192)
with open(dev_path, encoding="utf-8") as source, open(dest, "w", encoding="utf-8") as target:
    for line in source:
        if not line.strip():
            continue
        row = json.loads(line)
        for name, question in row["questions"].items():
            single = {"state": row["state"], "questions": {name: question},
                      "expected": {name: row["expected"][name]}}
            items, skipped = items_from_rows(tok, [single], max_len, head_max_len, label_smoothing=0.0)
            if len(items) != 1:
                raise ValueError("cannot score dev question %s: %s" % (name, skipped))
            records = calibration_records(model, tok, items, device, max_len, head_max_len,
                                          parallel=uses_parallel_layout(cfg))
            labels = list(question["criteria"])
            logits = records[0][1].tolist()
            target.write(json.dumps({"type": name, "label": row["expected"][name],
                                     "labels": labels, "logits": logits}) + "\n")
'''


def run(argv):
    # Only the final artifact map belongs on stdout.
    return subprocess.run(argv, stdout=sys.stderr).returncode


def fit(path, revision):
    groups = {}
    with path.open(encoding="utf-8") as source:
        for line in source:
            if not line.strip():
                continue
            row = json.loads(line)
            labels, logits = row["labels"], row["logits"]
            if (not isinstance(row["type"], str) or not labels or
                    any(not isinstance(x, str) for x in labels) or len(set(labels)) != len(labels) or
                    row["label"] not in labels or len(logits) != len(labels) or
                    any(not isinstance(x, (int, float)) or not math.isfinite(x) for x in logits)):
                raise ValueError("invalid dev_scores.jsonl row")
            entries = groups.setdefault(row["type"], [])
            if entries and entries[0][0] != labels:
                raise ValueError("inconsistent labels in dev_scores.jsonl for " + row["type"])
            entries.append((labels, labels.index(row["label"]), logits))
    result = {"format": 1, "model_version": "laya-ft-" + revision[:8], "types": {}}
    for name, rows in groups.items():
        def nll(temp):
            loss = 0.0
            for _, gold, logits in rows:
                scaled = [x / temp for x in logits]
                top = max(scaled)
                loss += top + math.log(sum(math.exp(x - top) for x in scaled)) - scaled[gold]
            return loss
        temperature = min((i / 20 for i in range(10, 101)), key=nll)
        predictions = []
        for _, gold, logits in rows:
            scaled = [x / temperature for x in logits]
            top = max(scaled)
            probabilities = [math.exp(x - top) for x in scaled]
            prediction = max(range(len(logits)), key=logits.__getitem__)
            predictions.append((probabilities[prediction] / sum(probabilities), prediction == gold))
        threshold = 0.95
        for i in range(10, 20):
            candidate = i / 20
            selected = [correct for confidence, correct in predictions if confidence >= candidate]
            if selected and sum(selected) / len(selected) >= 0.9:
                threshold = candidate
                break
        result["types"][name] = {"labels": rows[0][0], "temperature": temperature,
                                 "abstain_below": threshold, "fitted_on": "dev"}
    return result


def main():
    parents = Path(__file__).resolve().parents
    repo = parents[3] if len(parents) > 3 else parents[-1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--upstream", default=os.environ.get("AXIOM_LAYA_UPSTREAM", str(repo / ".laya" / "upstream")))
    parser.add_argument("--base", default="convaiinnovations/laya-multilingual")
    parser.add_argument("--revision")
    parser.add_argument("--epochs", type=int, default=3)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--device", choices=("cuda", "cpu"), default="cuda")
    args = parser.parse_args()
    if not args.revision:
        pins = repo / "tools" / "laya" / "pins.json"
        try:
            args.revision = json.loads(pins.read_text(encoding="utf-8"))["model"]["laya-multilingual"]["revision"]
        except (OSError, ValueError, KeyError, TypeError):
            pass
        if not args.revision:
            parser.error("missing model revision in pins.json; provide --revision")
    dataset, out, upstream = map(lambda p: Path(p).resolve(), (args.dataset, args.out, args.upstream))
    trainer = os.environ.get("AXIOM_LAYA_TRAINER")
    if trainer is not None:
        try:
            argv = json.loads(trainer)
            if not isinstance(argv, list) or not argv or any(not isinstance(x, str) for x in argv):
                raise ValueError("expected a nonempty JSON argv list")
        except ValueError as exc:
            parser.error("AXIOM_LAYA_TRAINER: " + str(exc))
        code = run(argv + ["--dataset", str(dataset), "--out", str(out), "--base", args.base,
                           "--revision", args.revision, "--epochs", str(args.epochs),
                           "--seed", str(args.seed), "--device", args.device])
    else:
        checkpoint = out / "checkpoint"
        code = run([sys.executable, "-c", TRAIN, str(upstream), args.base, args.revision,
                    str(dataset / "train.jsonl"), str(checkpoint), str(args.epochs), str(args.seed), args.device])
        if code:
            return code
        # Upstream exports <root>/<checkpoint basename>; use a temporary root inside OUT.
        export_root = out / "export"
        code = run([sys.executable, str(upstream / "laya-rust" / "tools" / "export_onnx.py"),
                    str(checkpoint), "--out", str(export_root)])
        if code:
            return code
        exported = export_root / checkpoint.name
        if exported.is_dir():
            shutil.move(str(exported), str(out / "onnx"))
        tokenizer = out / "onnx" / "tokenizer" / "tokenizer.json"
        if tokenizer.is_file():
            shutil.copyfile(tokenizer, out / "tokenizer.json")
        code = run([sys.executable, "-c", SCORE, str(upstream), str(checkpoint),
                    str(dataset / "dev.jsonl"), str(out / "dev_scores.jsonl"), args.device])
    if code:
        return code
    required = {"checkpoint/": (out / "checkpoint").is_dir() and any(p.is_file() for p in (out / "checkpoint").rglob("*")),
                "onnx/model.onnx": (out / "onnx" / "model.onnx").is_file(),
                "tokenizer.json": (out / "tokenizer.json").is_file(),
                "dev_scores.jsonl": (out / "dev_scores.jsonl").is_file()}
    for name, present in required.items():
        if not present:
            print("missing " + name, file=sys.stderr)
            return 3
    try:
        calibration = fit(out / "dev_scores.jsonl", args.revision)
    except (ValueError, KeyError, TypeError, OSError) as exc:
        print("invalid dev_scores.jsonl: " + str(exc), file=sys.stderr)
        return 3
    (out / "calibration.json").write_text(json.dumps(calibration, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"checkpoint": str(out / "checkpoint"), "onnx": str(out / "onnx"),
                      "tokenizer": str(out / "tokenizer.json"), "calibration": str(out / "calibration.json")}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
