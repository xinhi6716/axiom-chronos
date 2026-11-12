#!/bin/sh
set -e
BASE=$(python -c 'import json; print(json.load(open("manifest.json", encoding="utf-8"))["base"])')
REV=$(python -c 'import json; print(json.load(open("manifest.json", encoding="utf-8"))["revision"])')
echo 'Dependency install command: pip install -e "$AXIOM_LAYA_UPSTREAM" torch transformers huggingface_hub onnx onnxscript onnxruntime safetensors'
python run.py --dataset . --out ./out --base "$BASE" --revision "$REV" --epochs "${EPOCHS:-3}" --seed "${SEED:-1}" --device cuda
