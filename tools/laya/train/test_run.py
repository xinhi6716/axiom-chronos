import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[3]
RUN = ROOT / "tools" / "laya" / "train" / "run.py"
TRAINER = ROOT / "tests" / "fakes" / "fake_laya_trainer.py"
COLAB = ROOT / "tests" / "fakes" / "fake_colab.py"


class RunTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.dataset = self.root / "dataset"
        self.dataset.mkdir()
        self.out = self.root / "out"
        self.log = self.root / "trainer.jsonl"
        self.env = os.environ.copy()
        self.env.update(AXIOM_LAYA_TRAINER=json.dumps([sys.executable, str(TRAINER)]),
                        FAKE_LAYA_TRAINER_LOG=str(self.log))
        self.env.pop("FAKE_LAYA_TRAINER_FAIL", None)
        for name in ("train.jsonl", "calibration.jsonl", "dev.jsonl", "acceptance.jsonl", "datasheet.json"):
            (self.dataset / name).write_text("{}\n", encoding="utf-8")

    def invoke(self, *args):
        return subprocess.run([sys.executable, str(RUN), "--dataset", str(self.dataset),
                               "--out", str(self.out), *args], env=self.env, text=True, capture_output=True)

    def test_forward_outputs_and_calibration(self):
        revision = "abcdef1234567890"
        result = self.invoke("--base", "example/base", "--revision", revision,
                             "--epochs", "5", "--seed", "42", "--device", "cpu")
        self.assertEqual(result.returncode, 0, result.stderr)
        args = json.loads(self.log.read_text(encoding="utf-8"))
        self.assertEqual(args, ["--dataset", str(self.dataset), "--out", str(self.out),
                                "--base", "example/base", "--revision", revision,
                                "--epochs", "5", "--seed", "42", "--device", "cpu"])
        result_map = json.loads(result.stdout)
        self.assertEqual(set(result_map), {"checkpoint", "onnx", "tokenizer", "calibration"})
        self.assertTrue((Path(result_map["checkpoint"]) / "model.bin").is_file())
        self.assertTrue((Path(result_map["onnx"]) / "model.onnx").is_file())
        self.assertEqual(json.loads(Path(result_map["tokenizer"]).read_text(encoding="utf-8")), {})
        calibration = json.loads(Path(result_map["calibration"]).read_text(encoding="utf-8"))
        self.assertEqual(calibration["format"], 1)
        self.assertEqual(calibration["model_version"], "laya-ft-abcdef12")
        needs = calibration["types"]["needs"]
        self.assertEqual(needs["labels"], ["smoke", "manual", "none"])
        self.assertEqual(needs["fitted_on"], "dev")
        rows = [json.loads(line) for line in (self.out / "dev_scores.jsonl").read_text(encoding="utf-8").splitlines()]
        def nll(t):
            return sum(math.log(sum(math.exp((v - max(r["logits"])) / t) for v in r["logits"])) +
                       (max(r["logits"]) - r["logits"][r["labels"].index(r["label"])]) / t for r in rows)
        self.assertEqual(needs["temperature"], min((i / 20 for i in range(10, 101)), key=nll))
        self.assertEqual(needs["abstain_below"], 0.5)

    def test_missing_pin(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 2)
        self.assertIn("pins.json", result.stderr)
        self.assertFalse(self.log.exists())

    def test_trainer_failure(self):
        self.env["FAKE_LAYA_TRAINER_FAIL"] = "1"
        result = self.invoke("--revision", "abcdef123")
        self.assertEqual(result.returncode, 7)
        self.assertTrue(self.log.exists())
        self.assertFalse((self.out / "calibration.json").exists())

    def test_missing_dev_scores(self):
        script = ("import pathlib,subprocess,sys; "
                  "r=subprocess.run([sys.executable,sys.argv[1]]+sys.argv[2:]); "
                  "p=pathlib.Path(sys.argv[sys.argv.index('--out')+1])/'dev_scores.jsonl'; "
                  "p.unlink(); sys.exit(r.returncode)")
        self.env["AXIOM_LAYA_TRAINER"] = json.dumps([sys.executable, "-c", script, str(TRAINER)])
        result = self.invoke("--revision", "abcdef123")
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn("dev_scores.jsonl", result.stderr)
        self.assertFalse((self.out / "calibration.json").exists())

    def test_threshold_excludes_wrong_low_confidence_predictions(self):
        rows = [{"type": "needs", "label": "smoke", "labels": ["smoke", "manual"],
                 "logits": [4.0, 0.0]} for _ in range(8)]
        rows += [{"type": "needs", "label": "smoke", "labels": ["smoke", "manual"],
                  "logits": [0.0, 0.1]} for _ in range(3)]
        script = ("import json,pathlib,subprocess,sys; "
                  "r=subprocess.run([sys.executable,sys.argv[1]]+sys.argv[3:]); "
                  "p=pathlib.Path(sys.argv[sys.argv.index('--out')+1])/'dev_scores.jsonl'; "
                  "p.write_text(''.join(json.dumps(x)+'\\n' for x in json.loads(sys.argv[2])),encoding='utf-8'); "
                  "sys.exit(r.returncode)")
        self.env["AXIOM_LAYA_TRAINER"] = json.dumps([sys.executable, "-c", script, str(TRAINER), json.dumps(rows)])
        result = self.invoke("--revision", "abcdef123")
        self.assertEqual(result.returncode, 0, result.stderr)
        needs = json.loads((self.out / "calibration.json").read_text(encoding="utf-8"))["types"]["needs"]
        temp = needs["temperature"]
        wrong_confidence = 1 / (1 + math.exp(-0.1 / temp))
        correct_confidence = 1 / (1 + math.exp(-4.0 / temp))
        expected = next(i / 20 for i in range(10, 20)
                        if wrong_confidence < i / 20 <= correct_confidence)
        self.assertEqual(needs["abstain_below"], expected)

    def test_fake_colab(self):
        state = self.root / "state"
        self.env["FAKE_COLAB_STATE"] = str(state)
        bundle = self.root / "bundle"
        shutil.copytree(self.dataset, bundle)
        shutil.copyfile(RUN, bundle / "run.py")
        (bundle / "manifest.json").write_text(json.dumps({"base": "example/base", "revision": "abcdef123"}), encoding="utf-8")
        def call(*args):
            return subprocess.run([sys.executable, str(COLAB), *map(str, args)], env=self.env, text=True, capture_output=True)
        self.assertEqual(call("new").stdout.strip(), "sess-1")
        self.assertEqual(json.loads(call("usage").stdout)["compute_units"], 100.0)
        self.assertEqual(call("upload", "sess-1", bundle).returncode, 0)
        executed = call("exec", "sess-1", "ignored command")
        self.assertEqual(executed.returncode, 0, executed.stderr)
        self.assertEqual(json.loads(call("usage").stdout)["compute_units"], 98.5)
        trainer_args = json.loads(self.log.read_text(encoding="utf-8").splitlines()[-1])
        self.assertEqual(trainer_args[trainer_args.index("--base") + 1], "example/base")
        self.assertEqual(trainer_args[trainer_args.index("--revision") + 1], "abcdef123")
        self.env["FAKE_COLAB_EXEC_UNITS"] = "2.25"
        self.assertEqual(call("exec", "sess-1", "another command").returncode, 0)
        self.assertEqual(json.loads(call("usage").stdout)["compute_units"], 96.25)
        downloaded = self.root / "download"
        self.assertEqual(call("download", "sess-1", "out", downloaded).returncode, 0)
        self.assertTrue((downloaded / "calibration.json").is_file())
        self.assertEqual(call("close", "sess-1").returncode, 0)
        self.assertTrue((state / "closed").is_file())
        calls = [json.loads(line) for line in (state / "calls.jsonl").read_text(encoding="utf-8").splitlines()]
        self.assertEqual([x[0] for x in calls], ["new", "usage", "upload", "exec", "usage", "exec", "usage", "download", "close"])
        self.env["FAKE_COLAB_FAIL"] = "usage"
        self.assertEqual(call("usage").returncode, 9)
        self.assertEqual(json.loads((state / "calls.jsonl").read_text(encoding="utf-8").splitlines()[-1]), ["usage"])

    @unittest.skipUnless(os.name != "nt" and shutil.which("sh"), "POSIX shell unavailable")
    def test_colab_script_forwards_settings_and_failure(self):
        shutil.copyfile(RUN, self.dataset / "run.py")
        shutil.copyfile(RUN.parent / "run_colab.sh", self.dataset / "run_colab.sh")
        (self.dataset / "manifest.json").write_text(json.dumps({"base": "example/base", "revision": "abcdef123"}), encoding="utf-8")
        self.env.update(EPOCHS="2", SEED="13")
        result = subprocess.run([shutil.which("sh"), "run_colab.sh"], cwd=self.dataset, env=self.env,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("pip install", result.stdout)
        argv = json.loads(self.log.read_text(encoding="utf-8").splitlines()[-1])
        for flag, value in (("--base", "example/base"), ("--revision", "abcdef123"),
                            ("--epochs", "2"), ("--seed", "13"), ("--device", "cuda")):
            self.assertEqual(argv[argv.index(flag) + 1], value)
        self.env["FAKE_LAYA_TRAINER_FAIL"] = "1"
        failed = subprocess.run([shutil.which("sh"), "run_colab.sh"], cwd=self.dataset, env=self.env,
                                text=True, capture_output=True)
        self.assertEqual(failed.returncode, 7, failed.stderr)


if __name__ == "__main__":
    unittest.main()
