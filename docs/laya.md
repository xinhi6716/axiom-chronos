# Laya artifacts

Laya is off by default: without a data home it is off, and nothing loads unless
asked. It never starts Python or accesses the network at run time. Data home is
`AXIOM_DATA_HOME`, falling back to `CLAUDE_PLUGIN_DATA`. `AXIOM_LAYA_ORT_LIB`
can name a runtime library explicitly. On Windows `SystemRoot` determines the
inbox lookup location.

## Runtime lookup

The first existing library in this order is used:

1. `AXIOM_LAYA_ORT_LIB`.
2. `<home>/laya/router.json` field `runtime_lib`.
3. `<home>/laya/runtime/`.
4. `<repo>/.laya/runtime/` when a repo is supplied (for tests).
5. Windows inbox `%SystemRoot%\System32\onnxruntime.dll`.

The binding requests ONNX Runtime C API version 14; an older runtime without
that API fails with `laya_runtime_incompatible`.

## Fetch and pins

Run `python tools/laya/fetch.py --runtime --model laya-multilingual`. The
stdlib-only tool supports Windows and Linux x64 runtime archives. With neither
flag it fetches the model; `--dest <dir>` overrides the default `<repo>/.laya/`,
which is git-ignored. Runtime and model files land in `runtime/` and
`models/laya-multilingual/` below that directory.

`tools/laya/pins.json` records URLs, revisions, SHA-256 digests and byte sizes.
A missing digest is trusted on first use and recorded; inspect and review the
result before relying on it. The model's first recording resolves the commit
SHA via the Hugging Face model API, then uses commit-specific URLs. Later fetches
use those pins and refuse hash or size mismatches before installing artifacts.
The tool writes `axiom-laya.json` in the model directory with format 1, source,
revision, model version, detected fused/split layout and the SHA-256 for each
model file. The runtime also refuses files whose manifest SHA-256 does not match.

To refresh, review the desired upstream release, edit the runtime version and
archive URLs/revisions and clear the affected hashes, sizes and runtime file
entries. For a new model revision, clear its `revision`, `layout` and `files`;
rerun the fetch in a trusted network environment, review the resulting diff and
commit the pins. Run runtime fetches on both supported OSes to record both
archives and extracted files. First-use records are not independent proof of
upstream authenticity.

## Fetch status

On 2026-11-12 the requested real fetch was attempted on Windows x64. The ONNX
Runtime 1.20.1 archive request failed with connection refused (WinError 10061),
so no runtime or model was downloaded. A separate request to the Hugging Face
API was also refused. The model commit, file list, hashes and sizes and both
runtime archive/file hashes and sizes remain unrecorded in the pin skeleton;
a successful fetch on each platform is still required to populate them.
