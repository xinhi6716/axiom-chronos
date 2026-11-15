# Phase 8 paper record

## Acceptance

- Journal matching: the fixture ranks journals against Q1, OA and APC requirements and records metric provenance; without a JCR file quartiles are labelled SJR. `tests/paper_journals.rs`: `acceptance_fixture_order_scores_and_reasons`, `filter_cases_and_provenance`, `gather_merges_sources_by_issn_without_jcr`; `tests/paper_jcr.rs`: `quartiles_use_matching_issns_and_requested_or_newest_year`; `tests/paper_sources.rs`: `sjr_csv_semicolon_quartiles_labelled_sjr`.
- Templates: elsarticle and IEEEtran compile offline with a figure and a booktabs table: `tests/paper_templates.rs::compile_verified_classes_offline`; it is ignored with a reason when no TeX engine is found.
- Pipeline: four passes using fake members preserve claim-evidence links: `tests/paper_runner.rs::happy_path_records_prompts_events_files_and_state`, `unknown_claim_rejected`, `unknown_evidence_rejected`.
- Integrity gates: seeded fake citation and unsourced number block readiness: `tests/paper_review.rs::seeded_fake_doi_blocks_readiness` and `seeded_unsourced_number_blocks_readiness`; `tests/paper_integrity.rs::seeded_percentage_is_unsourced_located_and_blocking`; `tests/paper_citations.rs::fake_doi_and_unknown_key_block_manuscript`.
- Style: the humanizer is restricted to style, preserving claims, quantities and citations: `tests/paper_style.rs::style_accepts_wording_and_existing_literal`, `style_rejects_claim_change`, `style_rejects_quantity_change`, `style_rejects_citation_change`; `tests/paper_runner.rs::style_claim_change_rejected` and `style_citation_change_rejected`.
- Disclosure: `tests/paper_style.rs::disclosure_lists_only_successful_uses_and_policy` checks the generated statement and policy.
- Desktop UI: `tests/desktop-smoke/tests/paper.rs` covers entry, form, explicit journal choice, shortlist sources, four-pass progress, change logs and preview, including `write_paper_entry_opens_page`, `sources_and_unknowns_visible`, `four_pass_glyphs_correct`, and `real_pdf_preview_or_unavailable_reason`. The `PaperService` seam is wired in production to `SessionPaper`; `src/desktop/ux/real/paper_accept.rs::production_has_paper`, `fake_scripts_status_shortlist_failures_and_calls`, and `preview_sample_and_bad_page` cover the adapter and seam.
- Failing-first test commits, from `git log --oneline 7a5d43a9..HEAD`: `a3a9c2cf` foundation, `7e6451e3` evidence, `867f06ff` budget, `5f65fc3c` quantities, `f41ed425` style, `df98adf8` figures, `b09e7013` sources, `9a284ed2` citations, `de244fc8` runner, `9d40acc3` JCR, `680403a8` templates, `4e6272ba` render check, `7b32ff01` review, `59e1a763` service, `9b8fa1e0` desktop seam, `7d944388` desktop UI, `ef3c0fc4` gates. The runner and JCR log subjects say tests and stubs, without explicitly saying red.

## Always-loaded prompt bytes

The limit is 20,813 bytes; Astra measured 20,758 LF bytes before. The table measures `git show` at `7a5d43a9` and HEAD with CR bytes removed, equivalent to `tr -d '\r'`; Git's `tr` and `wc` failed with Windows error 5 here, so byte counting used Python on the raw `git show` output instead.

| File | before | after |
| --- | ---: | ---: |
| `engine/agents/dispatcher.md` | 9,862 | 9,862 |
| `engine/DISPATCH.md` | 2,824 | 2,824 |
| `engine/skills/wish/SKILL.md` | 8,072 | 8,072 |
| Total | 20,758 | 20,758 |

## Diff

`git diff --stat 7a5d43a9 -- tests/` summary: `133 files changed, 15416 insertions(+)`. Every change is a new file except the existing file listed by `git diff --stat --diff-filter=M 7a5d43a9 -- tests/`:

```text
 tests/desktop-smoke/Cargo.lock | 2 ++
 1 file changed, 2 insertions(+)
```

That lockfile adds the existing `ttf-parser` and `windows 0.61.3` packages to the axiom dependency list for the paper font and Windows PDF features. `git diff --stat 7a5d43a9 -- Cargo.lock Cargo.toml tests/desktop-smoke/Cargo.lock tests/desktop-smoke/Cargo.toml`:

```text
 Cargo.lock                     | 2 ++
 Cargo.toml                     | 4 +++-
 tests/desktop-smoke/Cargo.lock | 2 ++
 3 files changed, 7 insertions(+), 1 deletion(-)
```

## Figures and dependencies

Rust computes the physical layout and emits TikZ, compiled by Tectonic to vector PDF. Plotters was not added: it is absent from Cargo.lock, the build is offline, and text measured with `ttf-parser` supports the layout checks. Only line, scatter, error_bar and bar plots are implemented; other plot types, log axes and multi-panel layouts are not supported yet. See `docs/paper-figures.md` for the limits of the audits.

Runtime Tectonic resolution is `AXIOM_TECTONIC_CMD`, then `paper/settings.json`, then PATH. Tectonic 0.17.0 was present in a local toolchain folder on the development machine; the test probe also recognizes that location.

The direct dependencies are `ttf-parser` 0.25 and optional Windows `windows` 0.61.3, enabled by `app` with features `Data_Pdf`, `Foundation`, `Storage`, and `Storage_Streams`. Both were already in Cargo.lock. No new crates.

D5 XLSX outcome: JCR imports local XLSX as well as CSV. Its in-module ZIP/XML reader reads `xl/worksheets/sheet1.xml`, shared and inline strings and cell values; it uses the first row as headers and imports JCR records with file provenance. It supports stored and deflated ZIP entries, rejecting unsupported compression, encryption, ZIP64 and malformed archives. It does not resolve an arbitrary workbook sheet relationship; it specifically reads sheet1. `tests/paper_jcr.rs::xlsx_import_matches_csv` and `xlsx_rejects_truncation_garbage_encryption_and_zip64` cover this.

## Template evidence

The statuses in `engine/paper/templates/registry.json` are:

- elsarticle: verified using the Elsevier author page and Tectonic bundle class v3.3.
- IEEEtran: verified with weaker evidence, using the CTAN README and IEEE author center page; class V1.8b.
- sn-jnl: unconfirmed; the Springer page does not name the class.
- achemso: unconfirmed; ACS pages returned HTTP 403.

## Suite results

Runs use a local `CARGO_HOME`, `CARGO_BUILD_JOBS=2`, and `CARGO_INCREMENTAL=0`; outputs are in scratch. Counts exclude nested child-test summaries.

- `cargo test --locked --offline --no-fail-fast`: 1,812 passed, 135 failed, 5 ignored. The 134 baseline failures are exactly the named main-suite failures listed in `docs/integration-3.0.md`; all 134 failed again. The additional failure is `tests/scheduler_wall_clock.rs::scheduler_wall_clock_acceptance`, detailed below. The run used no `models_and_binary_configuration` skip.
- `cargo test --locked --offline --no-fail-fast --features app --lib desktop::`: 883 passed, 3 failed, 0 ignored. All three failures are in the desktop-lib baseline and reported Windows access denied (os error 5): `desktop::live::accept::service_snapshot_rebuild_gap_resync_and_disk_pause`, `desktop::live::tests::service_background_reattaches_hosted_session_keys`, and `desktop::ux::real::background_accept::detach_keeps_wishes_running_and_only_quit_stops_the_service`.
- In `tests/desktop-smoke`, `cargo test --locked --offline --no-fail-fast -j 1`: 140 passed, 6 failed, 1 ignored. All six failures are in the smoke baseline, in `tests/service_attach.rs`: `window_close_keeps_fake_wish_running_and_reattaches`, `quit_dialog_keep_working_leaves_service_running`, `login_start_row_reflects_and_toggles`, `quit_dialog_stop_and_quit_stops_wish_and_service`, `disk_pause_reaches_report`, and `quit_dialog_stays_open_when_service_is_gone`. All 15 paper UI tests passed. The ignored `a0_record_before` is an existing test in `ux_captures`, not a phase 8 paper test.
- The only ignored new phase 8 test in this run is `tests/paper_templates.rs::find_not_installed`: `engine on PATH: not_installed path untestable` (the build probe detects the available engine, including its test fallback path). The six template compile tests and `tests/paper_render.rs::real_vector_figure_and_manuscript` are conditionally ignored with `no TeX engine: set AXIOM_TECTONIC_CMD or put tectonic on PATH`; they ran here. No paper desktop-smoke tests were ignored.
- `cargo test --locked --offline --test paper_gates --test prompt_size --test engine --test installed_assets -- --skip required_phrases_are_present`: 39 passed, 0 failed, 0 ignored (one filtered out).

Ruling 23 requires that no test that passed at baseline fail now; the main baseline in `docs/integration-3.0.md` has 134 failures. Known baseline failures include `text_cases::required_phrases_are_present` (ruling 11 waiver) and `base::proc::tests::no_bare_command_new_outside_the_helper` (ruling 22 waiver; the offender must be only `src/desktop/ux/real/modes_accept.rs`). The observed procedure-guard failure names only that file.

## Failing gates

- `tests/scheduler_wall_clock.rs::scheduler_wall_clock_acceptance` failed outside the baseline list, contrary to ruling 23. Its measured T/N ratio was 1.8567114840596295, below the unchanged 2.0 threshold (wave 7301.75 ms/unit; scheduler 3932.62 ms/unit). No cause has been established.

## Limitations

- Springer and ACS preset DPI and sizes are unchecked. Real-font tests need Arial or DejaVu.
- A paper-service test returns early when Tectonic is absent (ruling 24 accepts this); Tectonic is present here, so the relevant tests can run.
- The agy stdin format is not yet checked against real agy.
- The temporary ruling 20 skip `--skip models_and_binary_configuration` on the paper-references ACCEPT line broke the "bundle" wording cycle and was subsequently removed. The final run has no such skip; a search confirms `engine/paper/prompts/revise.md` no longer contains the word `bundle`.
