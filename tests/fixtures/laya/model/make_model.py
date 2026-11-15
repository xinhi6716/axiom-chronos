"""Generate the self-contained Laya fixture next to this script (stdlib only)."""

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parent
TEST_SOURCE = '"""Stdlib-only checks for the generated Laya fixture."""\n\nimport hashlib\nimport json\nfrom pathlib import Path\nimport shutil\nimport subprocess\nimport sys\nimport tempfile\nimport unittest\n\nHERE = Path(__file__).resolve().parent\n\n\ndef fields(data):\n    """Read the varint and length-delimited protobuf fields used by this model."""\n    def varint(pos):\n        value = shift = 0\n        while True:\n            byte = data[pos]\n            pos += 1\n            value |= (byte & 127) << shift\n            if byte < 128:\n                return value, pos\n            shift += 7\n\n    pos = 0\n    while pos < len(data):\n        key, pos = varint(pos)\n        number, wire = key >> 3, key & 7\n        if wire == 0:\n            value, pos = varint(pos)\n        elif wire == 2:\n            size, pos = varint(pos)\n            value, pos = data[pos:pos + size], pos + size\n            if len(value) != size:\n                raise ValueError("truncated protobuf field")\n        else:\n            raise ValueError("unexpected protobuf wire type")\n        yield number, value\n\n\ndef values(data, number):\n    return [value for field, value in fields(data) if field == number]\n\n\ndef one(data, number):\n    result = values(data, number)\n    if len(result) != 1:\n        raise ValueError("expected exactly one field %d" % number)\n    return result[0]\n\n\ndef value_info(data):\n    tensor = one(one(data, 2), 1)\n    shape = one(tensor, 2)\n    return (one(data, 1).decode(), one(tensor, 1),\n            [one(dim, 2).decode() for dim in values(shape, 1)])\n\n\ndef check_hashes(directory):\n    manifest = json.loads((directory / "axiom-laya.json").read_bytes())\n    actual = {\n        path.relative_to(directory).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()\n        for path in directory.rglob("*")\n        if path.is_file() and path != directory / "axiom-laya.json"\n    }\n    return manifest["files"] == actual\n\n\nclass FixtureTests(unittest.TestCase):\n    def test_regeneration(self):\n        with tempfile.TemporaryDirectory() as temp:\n            dest = Path(temp) / "model"\n            dest.mkdir()\n            shutil.copyfile(HERE / "make_model.py", dest / "make_model.py")\n            subprocess.run([sys.executable, str(dest / "make_model.py")], cwd=temp, check=True)\n            expected = {p.relative_to(HERE) for p in HERE.rglob("*") if p.is_file()}\n            generated = {p.relative_to(dest) for p in dest.rglob("*") if p.is_file()}\n            self.assertEqual(expected, generated)\n            for name in expected:\n                self.assertEqual((HERE / name).read_bytes(), (dest / name).read_bytes(), str(name))\n\n    def test_onnx(self):\n        model = (HERE / "model.onnx").read_bytes()\n        self.assertEqual(one(model, 1), 7)\n        self.assertEqual(one(model, 2), b"axiom-laya-fixture")\n        opset = one(model, 8)\n        self.assertEqual(values(opset, 1), [])\n        self.assertEqual(one(opset, 2), 13)\n        graph = one(model, 7)\n        self.assertEqual([value_info(v) for v in values(graph, 11)], [\n            ("input_ids", 7, ["B", "S"]), ("attention_mask", 7, ["B", "S"]),\n            ("marker_pos", 7, ["B", "M"]), ("marker_mask", 9, ["B", "M"]),\n            ("qtype", 7, ["B"]),\n        ])\n        self.assertEqual([value_info(v) for v in values(graph, 12)], [\n            ("logits", 1, ["B", "M"]), ("act_logits", 1, ["B"]),\n        ])\n        nodes = values(graph, 1)\n        self.assertEqual([one(n, 4).decode() for n in nodes],\n                         ["Add", "GatherElements", "Cast", "Mul", "Cast"])\n        self.assertEqual([(values(n, 1), values(n, 2)) for n in nodes], [\n            ([b"marker_pos", b"one"], [b"idx"]),\n            ([b"input_ids", b"idx"], [b"gathered"]),\n            ([b"gathered"], [b"float_ids"]),\n            ([b"float_ids", b"scale"], [b"logits"]),\n            ([b"qtype"], [b"act_logits"]),\n        ])\n        for node, name, value in [(nodes[1], b"axis", 1), (nodes[2], b"to", 1),\n                                  (nodes[4], b"to", 1)]:\n            attr = one(node, 5)\n            self.assertEqual((one(attr, 1), one(attr, 3), one(attr, 20)), (name, value, 2))\n        tensors = {one(t, 8): t for t in values(graph, 5)}\n        self.assertEqual(set(tensors), {b"one", b"scale"})\n        self.assertEqual((one(tensors[b"one"], 2), one(tensors[b"one"], 9)),\n                         (7, b"\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00"))\n        self.assertEqual((one(tensors[b"scale"], 2), one(tensors[b"scale"], 9)),\n                         (1, b"\\xcd\\xcc\\xcc\\x3d"))\n\n    def test_tokenizer_and_config(self):\n        tokenizer = json.loads((HERE / "tokenizer.json").read_bytes())\n        vocab = tokenizer["model"]["vocab"]\n        self.assertEqual([vocab[token] for token in\n                          ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]", "choice",\n                           "question", ":", "amend", "answer"]], list(range(10)))\n        for label in "stop smoke manual none blocker major minor terra sol astra".split():\n            self.assertIn(label, vocab)\n        self.assertEqual(len(vocab), len(set(vocab.values())))\n        config = json.loads((HERE / "rl_agent_config.json").read_bytes())\n        self.assertEqual(config, {"max_len": 64, "head_max_len": 32,\n                                  "temperature": [1.0, 1.0, 1.0]})\n\n    def test_manifest_and_negative_control(self):\n        manifest = json.loads((HERE / "axiom-laya.json").read_bytes())\n        self.assertEqual({k: v for k, v in manifest.items() if k != "files"}, {\n            "format": 1, "layout": "fused", "model_version": "fixture-1",\n            "revision": "fixture-1", "source": "fixture",\n        })\n        self.assertTrue(check_hashes(HERE))\n        with tempfile.TemporaryDirectory() as temp:\n            dest = Path(temp) / "model"\n            shutil.copytree(HERE, dest)\n            path = dest / "model.onnx"\n            model = bytearray(path.read_bytes())\n            model[len(model) // 2] ^= 1\n            path.write_bytes(model)\n            self.assertFalse(check_hashes(dest))\n\n\nif __name__ == "__main__":\n    unittest.main()\n'


def varint(value):
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def integer(field, value):
    return varint(field << 3) + varint(value)


def blob(field, value):
    if isinstance(value, str):
        value = value.encode("utf-8")
    return varint((field << 3) | 2) + varint(len(value)) + value


def attribute(name, value):
    # AttributeProto: name, i, type=INT.
    return blob(1, name) + integer(3, value) + integer(20, 2)


def node(op, inputs, output, attr=None):
    return (b"".join(blob(1, item) for item in inputs) + blob(2, output) + blob(4, op)
            + (blob(5, attribute(*attr)) if attr else b""))


def tensor(name, dtype, raw):
    # Scalar TensorProto (no dims), raw little-endian data.
    return integer(2, dtype) + blob(8, name) + blob(9, raw)


def value_info(name, dtype, dims):
    shape = b"".join(blob(1, blob(2, dim)) for dim in dims)
    tensor_type = integer(1, dtype) + blob(2, shape)
    return blob(1, name) + blob(2, blob(1, tensor_type))


def model_bytes():
    nodes = [
        node("Add", ["marker_pos", "one"], "idx"),
        node("GatherElements", ["input_ids", "idx"], "gathered", ("axis", 1)),
        node("Cast", ["gathered"], "float_ids", ("to", 1)),
        node("Mul", ["float_ids", "scale"], "logits"),
        node("Cast", ["qtype"], "act_logits", ("to", 1)),
    ]
    inputs = [("input_ids", 7, ["B", "S"]), ("attention_mask", 7, ["B", "S"]),
              ("marker_pos", 7, ["B", "M"]), ("marker_mask", 9, ["B", "M"]),
              ("qtype", 7, ["B"])]
    outputs = [("logits", 1, ["B", "M"]), ("act_logits", 1, ["B"])]
    graph = (b"".join(blob(1, n) for n in nodes) + blob(2, "laya_fixture")
             + blob(5, tensor("one", 7, struct.pack("<q", 1)))
             + blob(5, tensor("scale", 1, struct.pack("<f", 0.1)))
             + b"".join(blob(11, value_info(*v)) for v in inputs)
             + b"".join(blob(12, value_info(*v)) for v in outputs))
    # ModelProto: IR version, producer, graph, default-domain opset import.
    return integer(1, 7) + blob(2, "axiom-laya-fixture") + blob(7, graph) + blob(8, integer(2, 13))


def write_json(name, obj):
    (ROOT / name).write_bytes((json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=False) + "\n").encode("utf-8"))


def main():
    words = ("[PAD] [UNK] [CLS] [SEP] [MASK] choice question : amend answer "
             "stop smoke manual none blocker major minor terra sol astra "
             "the a is fix test build wish user says please now add remove change file "
             "code run check review error bug ok yes no and or to of in it this that "
             "with for not do done plan task").split()
    assert len(words) == len(set(words))
    vocab = {word: i for i, word in enumerate(words)}
    tokenizer = {
        "version": "1.0", "truncation": None, "padding": None,
        "added_tokens": [{"id": i, "content": word, "single_word": False,
                          "lstrip": False, "rstrip": False, "normalized": False, "special": True}
                         for i, word in enumerate(words[:5])],
        "normalizer": {"type": "BertNormalizer", "clean_text": True,
                       "handle_chinese_chars": True, "strip_accents": None, "lowercase": True},
        "pre_tokenizer": {"type": "BertPreTokenizer"},
        "post_processor": {
            "type": "TemplateProcessing",
            "single": [{"SpecialToken": {"id": "[CLS]", "type_id": 0}},
                       {"Sequence": {"id": "A", "type_id": 0}},
                       {"SpecialToken": {"id": "[SEP]", "type_id": 0}}],
            "pair": [{"SpecialToken": {"id": "[CLS]", "type_id": 0}},
                     {"Sequence": {"id": "A", "type_id": 0}},
                     {"SpecialToken": {"id": "[SEP]", "type_id": 0}},
                     {"Sequence": {"id": "B", "type_id": 1}},
                     {"SpecialToken": {"id": "[SEP]", "type_id": 1}}],
            "special_tokens": {token: {"id": token, "ids": [vocab[token]], "tokens": [token]}
                               for token in ("[CLS]", "[SEP]")},
        },
        "decoder": {"type": "WordPiece", "prefix": "##", "cleanup": True},
        "model": {"type": "WordPiece", "unk_token": "[UNK]", "continuing_subword_prefix": "##",
                  "max_input_chars_per_word": 100, "vocab": vocab},
    }
    (ROOT / "model.onnx").write_bytes(model_bytes())
    write_json("tokenizer.json", tokenizer)
    write_json("rl_agent_config.json", {"max_len": 64, "head_max_len": 32,
                                       "temperature": [1.0, 1.0, 1.0]})
    readme = ("# Laya fixture\n\n"
              "Generated by `python make_model.py` from any working directory; stdlib only.\n\n"
              "For valid marker positions, `logits[b,m] = 0.1 * float(input_ids[b, marker_pos[b,m] + 1])` "
              "and `act_logits[b] = float(qtype[b])`. The attention and marker masks are declared "
              "but unused; callers must supply valid indices, including for padded markers.\n\n"
              "## Vocabulary IDs\n\n" + "\n".join(f"- {i}: `{word}`" for i, word in enumerate(words)) + "\n")
    (ROOT / "README.md").write_bytes(readme.encode("utf-8"))
    (ROOT / "test_fixture.py").write_bytes(TEST_SOURCE.encode("utf-8"))
    files = {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in ROOT.rglob("*") if p.is_file() and p != ROOT / "axiom-laya.json"}
    write_json("axiom-laya.json", {"files": files, "format": 1, "layout": "fused",
                                   "model_version": "fixture-1", "revision": "fixture-1",
                                   "source": "fixture"})


if __name__ == "__main__":
    main()
