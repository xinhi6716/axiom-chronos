# Paper figure rendering and quality

D6 route: Rust computes the physical layout in millimetres and emits a standalone TikZ document for each figure. Tectonic compiles it to a vector PDF with embedded fonts. Plotters is not used; there is no PDF writer. The vector PDF is the publication artifact: preview bitmaps never replace the vector PDF.

Ruling 4 uses fixed thresholds: text boxes have zero intersection area, stay within the canvas, and use at least the preset minimum font size; axis labels include the data unit and the chosen font covers every glyph. Metadata boxes are excluded. Tables must fit the text width and include each column's unit in its header. Missing fonts and missing glyphs are reported.

The page check reads the PDF page tree, including xref and object streams. An image drawn over at least 90% of the page MediaBox is reported as raster; it also reports page counts outside the requested range and each XeTeX missing-character log line. Missing, truncated or unreadable PDFs fail the check. No VLM page check exists; these mechanical checks do not assess visual or scientific publication quality.

The following records the `paper_render` tests on this environment. Clean simulated audits use `FixedMeasure`, which assumes glyph coverage; they are not a real-font audit. The real Tectonic test checks a line-figure PDF and a manuscript containing the booktabs table, not separate PDFs for every type. The synthetic page tests verify vector/raster detection and negative cases, not the visual quality of each type.

| Type | Audit result | Page result | Quality |
| --- | --- | --- | --- |
| line | Clean with FixedMeasure | Real one-page vector figure passes | Automated checks pass; publication quality not independently established |
| scatter | Clean with FixedMeasure | Individual PDF not checked | Publication quality not established: no per-type PDF or visual check |
| error_bar | Clean with FixedMeasure | Individual PDF not checked | Publication quality not established: no per-type PDF or visual check |
| bar | Clean with FixedMeasure | Individual PDF not checked | Publication quality not established: no per-type PDF or visual check |
| booktabs | Width and unit audit clean | Included in real manuscript PDF; page check passes | Automated checks pass; publication quality not independently established |
| CJK | Clean with FixedMeasure; missing-font and missing-glyph controls reported | Individual CJK PDF not checked | Publication quality not established: requires a present CJK font file with glyph coverage and a real PDF check |
