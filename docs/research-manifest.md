# Research workspace manifest (version 1)

A project is located by its root `research-manifest.json`, or by the read-only RRAM adapter when `data/literature_inventory.csv` and `research/protocol.yaml` exist. A file manifest has precedence. Paths use forward slashes; artifact and inventory paths are relative to their selected root. `project` is implicit. Named roots may be absolute or relative to the project; they are read-only. Traversal, absolute artifact paths, drive prefixes and symlink/junction components are rejected. Nothing is written to the research project by the adapter.

## Schema

Enums use snake_case. The root object contains:

- `manifest_version` (integer, 1), `research_id` (slug), `research_version`, `title`, and optional `session`.
- `roots`: object mapping names to paths; `capabilities`: capability array; `settings`: opaque object.
- `versions`: optional `code`, maps `prompts` and `environment` to strings, and `seeds` to integers.
- `artifacts`, `stages`, `links`, optional `groups`, and optional `stall_after_s`.

An artifact has `id`, `role`, optional `root`, relative `path`, `format` (`csv`, `jsonl`, `json`, `text`), optional declared `sha256`, `columns`, optional `documents_root`, and optional `produced`. Roles are `inventory`, `screening`, `aliases`, `evidence`, `measurements`, `raw_measurements`, `progress`, `driver`, `completed`, `pilot`, `acceptance`, `splits`, `validation`, `gate`, `protocol`, `raw_summary`, and `other`. A declared hash mismatch is corruption.

`columns` maps canonical names to actual CSV headers or JSON pointers. Unmapped CSV names are retained. `produced` has `inputs`, optional `code_version`, `prompt_version`, and `at`. Each input has `artifact` and optional `sha256` or `sha256_from`: an artifact id naming a text file containing the hex, or `<artifact id>#<json pointer>`.

A stage has `id`, `label`, optional progress artifact id, `outputs` artifact ids and optional `gate`. A gate specifies `artifact`, JSON `pointer`, `op` (`eq`, `ge`, `le`) and either a `value` or a `setting` JSON pointer into settings. A link has `from`, `to`, canonical `key` in the destination, and optional `separator`. Groups name the `pilot` and `acceptance` artifacts and have `by` either `"study_groups"` or `{"column":"lab"}` (with the desired column name). Record identifiers are `<artifact>:<key>`.

## Canonical columns

An asterisk marks the key. Fields may be empty; mappings do not imply that data exists.

| Artifact | Canonical fields |
| --- | --- |
| inventory | path*, file_name, bytes, sha256, n_pages, doi, title, doc_type, duplicate_group, duplicate_type, canonical, paper_id, supplement_of, erratum_of, include, exclusion_reason, read_error |
| screening | sha256*, paper_id, path, route, verdict (include/exclude/pending), reason, evidence_type, evidence_page, evidence_quote, quote_verified, pending_reason |
| aliases | category, alias, canonical, match; key is `<category>\|<alias>` |
| evidence | evidence_id*, paper_id, sha256, physical_page, printed_page, page_offset, locator_type, locator_id, quote, quote_verified, bbox, page_width, page_height, coordinate_origin, coordinate_units, rotation, raw_file, raw_root, raw_row, raw_key, raw_column |
| measurements | row_id*, paper_id, sha256, device_id, condition_id, scope, kind, variable, entity_raw, entity_canonical, value_original, unit_original, value_num, value_canonical, unit_canonical, conversion_rule, missing_reason, quality_grade, evidence_id (semicolon-separated), flags |
| raw_measurements | row_id*, source_file, source_row, source_key, source_column, device_id, condition_id, variable, entity_raw, entity_canonical, value_original, unit_original, value_canonical, unit_canonical, conversion_rule, missing_reason, quality_grade, evidence_id |
| completed (JSONL) | stage, sha256, status, attempt, retry_after, error, ts |
| pilot, acceptance | paper_id, optional group column |
| progress (JSON) | stage, total, done, failed, pending, rate_limited_until (`unknown`, RFC 3339, or null), updated |
| driver (JSON) | state, stage, reason, retry_at, updated, heartbeat |

Evidence locator geometry is optional: `bbox` is `x0,y0,x1,y1`; `page_width` and `page_height` describe the page; `coordinate_origin` is `top_left` or `bottom_left`; `coordinate_units` is `pt`, `px`, or `normalized`; `rotation` is 0, 90, 180 or 270 degrees. Raw-field locators use `raw_file`, `raw_root`, `raw_row`, `raw_key` and `raw_column`.

## Capabilities and RRAM

File manifests declare capabilities explicitly: `pdf_pages`, `raw_measurements`, `literature`, `screening`, `evidence`, `dataset`, `pilot`, `acceptance`, `splits`, `progress`. Requiring an absent capability returns `not_capable` naming it.

The RRAM adapter declares only existing artifacts. `pdf_pages` requires inventory and evidence; `raw_measurements` requires the raw table, not merely a summary; `literature`, `screening`, `evidence`, `pilot`, `acceptance`, and `splits` require their respective artifacts; `dataset` requires measurements or raw measurements; `progress` requires progress, driver or completed. Without the raw table the ingest summary does not imply tabulated raw values. The adapter uses protocol settings and seeds, the newest snapshot stamp, prompt hashes, the local HEAD/ref, and the Python requirement if present. Its limited flow-style YAML reader records unread top-level keys in `settings._unread`; it does not guess them.

CSV records default to a 1 MiB limit, pages to 500 rows, JSON/text to 16 MiB. Page cursors bind artifact, byte count and modification time; a changed source yields `stale_cursor`. JSONL parse errors, including a truncated final line, name the line.

## Error kinds

`unknown_version`, `invalid_manifest`, `no_manifest`, `missing_artifact`, `corrupt_artifact`, `oversized_record`, `stale_cursor`, `escape`, `not_capable`, `invalid_argument`, `dest_in_source`, `renderer_missing`, `corrupt_pdf`, `hash_mismatch`, `page_out_of_range`, `io`.

## Follow-ups (v2 and later)

Precise bbox highlighting (2 px at 100%), snapshot diff, model and proposal detail, paper and memory wiring, interactive DAG, collaboration sync, visual polish, and GPU ledger.

<!-- research workspace (phase 9) -->
## Measured performance

Filled by rw-gates.
<!-- end research workspace (phase 9) -->
