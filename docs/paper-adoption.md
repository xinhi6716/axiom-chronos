# Paper reference adoption

Decisions below are based on the pinned README and licence files or the named local skills. The only vendored asset is `paper/academic-humanizer`, read on demand via `engine::asset("paper/academic-humanizer/SKILL.md")`; the nature-* skills are not copied.

### AIScientists-Dev/academic-humanizer
Source: https://github.com/AIScientists-Dev/academic-humanizer
Version: 94b88b23703bed7df507acae7d6d5876209a0cdf
Licence: MIT
Decision: adopt
Reason: The README describes a clarity and voice editing pass that preserves claims, numbers and citations and does not bypass disclosure.

### Imbad0202/academic-research-skills
Source: https://github.com/Imbad0202/academic-research-skills
Version: c67a7be53f23d9dbba4030bbcd6f5ccfeda3afa9
Licence: CC-BY-NC-4.0
Decision: reference-only
Reason: The README presents a research-to-publication suite and a Socratic paper planning workflow; its noncommercial licence limits reuse.

### jin-s13/ai-research-writing-skill
Source: https://github.com/jin-s13/ai-research-writing-skill
Version: e8b5de964737350757b5b17d8a658a22943c9595
Licence: MIT
Decision: adapt
Reason: The README maps major claims to code, results, notes or verified citations. Adapt that claim-evidence map for the model council and Tectonic workflow.

### appautomaton/latex-arxiv-SKILL
Source: https://github.com/appautomaton/latex-arxiv-SKILL
Version: b5b058350796bd6c9df7725a5d2e1607a9d45b2a
Licence: MIT
Decision: adapt
Reason: The README combines issue-based writing, citation verification and compilation. Adapt the citation and compile checks to Tectonic rather than its Python and pdflatex harness.

### yunshenwuchuxun/latex-paper-skills
Source: https://github.com/yunshenwuchuxun/latex-paper-skills
Version: 7965fbc06cd2c34ef7a0a3a566c089f4757c3de5
Licence: MIT
Decision: reject
Reason: The README describes a portable ML paper bundle with verified BibTeX and multi-agent workflows; its complete external harness duplicates the app’s model council and build flow.

### bahayonghang/academic-writing-skills
Source: https://github.com/bahayonghang/academic-writing-skills
Version: 08a7b34a3575cbd7de9cfd442e01e38dfd9719d6
Licence: PolyForm-Noncommercial-1.0.0
Decision: reference-only
Reason: The README describes post-writing format, bibliography and experiment narrative checks, with human verification of audit reports; the licence is noncommercial.

### Zhangyanbo/vibe-paper-writing
Source: https://github.com/Zhangyanbo/vibe-paper-writing
Version: 3dc57bf350c84fac3c7717143ce394c0e3332dc1
Licence: MIT
Decision: reject
Reason: The README describes converting conversations into LaTeX prose and compiling after substantive edits; conversational auto-writing is not needed for the evidence-driven app.

### Rezenders/scientific_writing_agent
Source: https://github.com/Rezenders/scientific_writing_agent
Version: d8c1eb5be8a6d3a632f1a6231f23d4112ba4c8eb
Licence: Apache-2.0
Decision: adapt
Reason: The README separates reviewers who diagnose from authors controlling substantive revisions. Adapt that review boundary to the council without replacing the LaTeX build system.

### K-Dense-AI/scientific-agent-skills
Source: https://github.com/K-Dense-AI/scientific-agent-skills
Version: 2d2065fb4520fa3f3f2bcf61cff26ec8b7eba4b5
Licence: MIT
Decision: reject
Reason: The README offers a broad scientific skill library and desktop workspace with many research integrations; importing the full collection is unnecessary for the focused paper workflow.

### Plusero/agentic-research-skills
Source: https://github.com/Plusero/agentic-research-skills
Version: 461e3d07b49d2e99ac58ae189e55d2f6598da90d
Licence: CC0-1.0
Decision: reference-only
Reason: The CC0 1.0 licence permits copying, but this catalogue of skills is reference-only because the paper workflow does not need to copy it.

### y9c/scifig
Source: https://github.com/y9c/scifig
Version: 9edcae657738decdeedbb95b1652f98c27b300c5
Licence: MIT
Decision: adapt
Reason: The README makes publication figures with plain matplotlib and examples rather than a package. Adapt its reproducible source-to-figure approach without adding a Python dependency.

### giacolees/scifig
Source: https://github.com/giacolees/scifig
Version: 410547be33dd323fd8b04d3111b0fa49bc157aee
Licence: MIT
Decision: adapt
Reason: The README describes reproducible Python or YAML plots and architecture diagrams with shared validation and export. Adapt the explicit figure specification and validation idea, not its library.

### yanggaihao5-sys/dsh-sci-figures
Source: https://github.com/yanggaihao5-sys/dsh-sci-figures
Version: c265609bf459fcf6399f1c802a4b16aef172f2f6
Licence: MIT
Decision: adapt
Reason: The README starts with what a figure must prove and discusses colour-blind-safe palettes and vector fonts. Adapt the figure-message and accessibility checks to the plan.

### ACG-j/academic-skills
Source: https://github.com/ACG-j/academic-skills
Version: 3462ae9064a83c1125733987b62a4174aabc31d5
Licence: none found at 3462ae9064a83c1125733987b62a4174aabc31d5
Decision: reference-only
Reason: The README separates static, runtime layout and visual figure checks; no licence was found at this commit.

### WUWeifeng710/make-my-figure-skill
Source: https://github.com/WUWeifeng710/make-my-figure-skill
Version: 193ea0d82f4a1de7ec0fa1460f06dc832989325b
Licence: MIT
Decision: reject
Reason: The README describes a local plotting engine with many journal styles and JSON sidecars. Its separate plotting stack is outside the focused Tectonic and council app.

### vladimirc777/journal-finder
Source: https://github.com/vladimirc777/journal-finder
Version: none
Licence: unknown
Decision: not evaluated: fetch failed 2026-11-13
Reason: The GitHub API returned 404, so no README or licence was available for evaluation.

### ikhwan-arief/journal-indexer-scientific-discovery
Source: https://github.com/ikhwan-arief/journal-indexer-scientific-discovery
Version: 69b6c86919a836207e52ce87a8097b00ae8be437
Licence: none found at 69b6c86919a836207e52ce87a8097b00ae8be437
Decision: reference-only
Reason: The README describes journal discovery using DOAJ and Scimago data and OpenAlex matching. No licence was found at this commit.

### nature-figure
Source: C:/Users/gppup/.claude/skills/nature-figure/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Check figure layout and visual quality against the intended message.

### nature-writing
Source: C:/Users/gppup/.claude/skills/nature-writing/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Build an evidence chain linking manuscript claims to supplied evidence and boundaries.

### nature-polishing
Source: C:/Users/gppup/.claude/skills/nature-polishing/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Preserve facts and evidence boundaries while refining prose.

### nature-statistics
Source: C:/Users/gppup/.claude/skills/nature-statistics/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Report experimental units, replication, uncertainty and inference transparently.

### nature-citation
Source: C:/Users/gppup/.claude/skills/nature-citation/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Map claims to verified supporting sources.

### nature-ref-verifier
Source: C:/Users/gppup/.claude/skills/nature-ref-verifier/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Verify bibliographic records against reliable sources rather than assuming citations are correct.

### nature-academic-search
Source: C:/Users/gppup/.claude/skills/nature-academic-search/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Search across sources and verify citations using the appropriate literature workflow.

### nature-paper2word
Source: C:/Users/gppup/.claude/skills/nature-paper2word/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Check formatting requirements and layout before and after export.

### nature-response
Source: C:/Users/gppup/.claude/skills/nature-response/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Map each reviewer response and claimed manuscript change to its location.

### nature-reviewer
Source: C:/Users/gppup/.claude/skills/nature-reviewer/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Freeze independently isolated reviews before synthesis and ground concerns in evidence.

### nature-data
Source: C:/Users/gppup/.claude/skills/nature-data/SKILL.md
Version: local (undated)
Licence: local personal skill
Decision: reference-only
Reason: Map datasets to access routes and identifiers without inventing availability details.
