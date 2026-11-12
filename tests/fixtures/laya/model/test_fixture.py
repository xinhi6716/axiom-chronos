"""Stdlib-only checks for the generated Laya fixture."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def fields(data):
    """Read the varint and length-delimited protobuf fields used by this model."""
    def varint(pos):
        value = shift = 0
        while True:
            byte = data[pos]
            pos += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value, pos
            shift += 7

    pos = 0
    while pos < len(data):
        key, pos = varint(pos)
        number, wire = key >> 3, key & 7
        if wire == 0:
            value, pos = varint(pos)
        elif wire == 2:
            size, pos = varint(pos)
            value, pos = data[pos:pos + size], pos + size
            if len(value) != size:
                raise ValueError("truncated protobuf field")
        else:
            raise ValueError("unexpected protobuf wire type")
        yield number, value


def values(data, number):
    return [value for field, value in fields(data) if field == number]


def one(data, number):
    result = values(data, number)
    if len(result) != 1:
        raise ValueError("expected exactly one field %d" % number)
    return result[0]


def value_info(data):
    tensor = one(one(data, 2), 1)
    shape = one(tensor, 2)
    return (one(data, 1).decode(), one(tensor, 1),
            [one(dim, 2).decode() for dim in values(shape, 1)])


def check_hashes(directory):
    manifest = json.loads((directory / "axiom-laya.json").read_bytes())
    actual = {
        path.relative_to(directory).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in directory.rglob("*")
        if path.is_file() and path != directory / "axiom-laya.json"
    }
    return manifest["files"] == actual


class FixtureTests(unittest.TestCase):
    def test_regeneration(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / "model"
            dest.mkdir()
            shutil.copyfile(HERE / "make_model.py", dest / "make_model.py")
            subprocess.run([sys.executable, str(dest / "make_model.py")], cwd=temp, check=True)
            expected = {p.relative_to(HERE) for p in HERE.rglob("*") if p.is_file()}
            generated = {p.relative_to(dest) for p in dest.rglob("*") if p.is_file()}
            self.assertEqual(expected, generated)
            for name in expected:
                self.assertEqual((HERE / name).read_bytes(), (dest / name).read_bytes(), str(name))

    def test_onnx(self):
        model = (HERE / "model.onnx").read_bytes()
        self.assertEqual(one(model, 1), 7)
        self.assertEqual(one(model, 2), b"axiom-laya-fixture")
        opset = one(model, 8)
        self.assertEqual(values(opset, 1), [])
        self.assertEqual(one(opset, 2), 13)
        graph = one(model, 7)
        self.assertEqual([value_info(v) for v in values(graph, 11)], [
            ("input_ids", 7, ["B", "S"]), ("attention_mask", 7, ["B", "S"]),
            ("marker_pos", 7, ["B", "M"]), ("marker_mask", 9, ["B", "M"]),
            ("qtype", 7, ["B"]),
        ])
        self.assertEqual([value_info(v) for v in values(graph, 12)], [
            ("logits", 1, ["B", "M"]), ("act_logits", 1, ["B"]),
        ])
        nodes = values(graph, 1)
        self.assertEqual([one(n, 4).decode() for n in nodes],
                         ["Add", "GatherElements", "Cast", "Mul", "Cast"])
        self.assertEqual([(values(n, 1), values(n, 2)) for n in nodes], [
            ([b"marker_pos", b"one"], [b"idx"]),
            ([b"input_ids", b"idx"], [b"gathered"]),
            ([b"gathered"], [b"float_ids"]),
            ([b"float_ids", b"scale"], [b"logits"]),
            ([b"qtype"], [b"act_logits"]),
        ])
        for node, name, value in [(nodes[1], b"axis", 1), (nodes[2], b"to", 1),
                                  (nodes[4], b"to", 1)]:
            attr = one(node, 5)
            self.assertEqual((one(attr, 1), one(attr, 3), one(attr, 20)), (name, value, 2))
        tensors = {one(t, 8): t for t in values(graph, 5)}
        self.assertEqual(set(tensors), {b"one", b"scale"})
        self.assertEqual((one(tensors[b"one"], 2), one(tensors[b"one"], 9)),
                         (7, b"\x01\x00\x00\x00\x00\x00\x00\x00"))
        self.assertEqual((one(tensors[b"scale"], 2), one(tensors[b"scale"], 9)),
                         (1, b"\xcd\xcc\xcc\x3d"))

    def test_tokenizer_and_config(self):
        tokenizer = json.loads((HERE / "tokenizer.json").read_bytes())
        vocab = tokenizer["model"]["vocab"]
        self.assertEqual([vocab[token] for token in
                          ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]", "choice",
                           "question", ":", "amend", "answer"]], list(range(10)))
        for label in "stop smoke manual none blocker major minor terra sol astra".split():
            self.assertIn(label, vocab)
        self.assertEqual(len(vocab), len(set(vocab.values())))
        config = json.loads((HERE / "rl_agent_config.json").read_bytes())
        self.assertEqual(config, {"max_len": 64, "head_max_len": 32,
                                  "temperature": [1.0, 1.0, 1.0]})

    def test_manifest_and_negative_control(self):
        manifest = json.loads((HERE / "axiom-laya.json").read_bytes())
        self.assertEqual({k: v for k, v in manifest.items() if k != "files"}, {
            "format": 1, "layout": "fused", "model_version": "fixture-1",
            "revision": "fixture-1", "source": "fixture",
        })
        self.assertTrue(check_hashes(HERE))
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / "model"
            shutil.copytree(HERE, dest)
            path = dest / "model.onnx"
            model = bytearray(path.read_bytes())
            model[len(model) // 2] ^= 1
            path.write_bytes(model)
            self.assertFalse(check_hashes(dest))


if __name__ == "__main__":
    unittest.main()
