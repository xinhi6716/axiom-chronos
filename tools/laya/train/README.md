# Laya fine-tuning

Set `model.laya-multilingual.revision` in `tools/laya/pins.json` to the immutable Hugging Face commit SHA, or supply `--revision SHA`. The default pin is null and is deliberately rejected. Install the upstream Laya checkout and its training/export dependencies before running. `AXIOM_LAYA_UPSTREAM` selects the checkout (default `.laya/upstream`); `--upstream` overrides it.

Local run from the repository root:

```sh
python tools/laya/train/run.py --dataset DATASET_DIR --out OUTPUT_DIR --revision SHA --device cpu
```

The dataset directory must contain `train.jsonl`, `calibration.jsonl`, `dev.jsonl`, `acceptance.jsonl`, and `datasheet.json`. Training uses only `train.jsonl`. The fitted calibration uses dev scores, not the training or acceptance set; acceptance remains independent. Outputs include `checkpoint/`, `onnx/`, `tokenizer.json`, `dev_scores.jsonl`, and `calibration.json`.

For Colab, upload the bundle containing `run.py`, `run_colab.sh`, `manifest.json` (with `base` and `revision`), and the five dataset files. In the bundle directory set up the pinned upstream checkout and dependencies, then run `sh run_colab.sh`. The script prints the dependency installation command without running it. Set `EPOCHS` and `SEED` to override defaults. Download `out/` after completion.

Choose explicit time, GPU and compute-unit caps before starting a local or Colab run; this wrapper does not enforce caps. Monitor Colab usage and stop the session at the cap. When finished, close the Colab session and delete remote bundles, datasets and outputs as appropriate; remove local checkpoints and caches when no longer needed.
