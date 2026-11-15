use axiom::{
    paper::{
        journals::{
            jcr::{self, JcrImport, JcrRecord},
            xlsx,
        },
        model::{self, Quartile, QuartileScheme, QuartileValue, SourceKind, SourceTag, Sourced},
    },
    seams::sha256_hex,
};
use flate2::{write::DeflateEncoder, Compression, Crc};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/paper/jcr")
        .join(name)
}

// Compare the public fields without requiring additional traits on the contract types.
fn assert_records(actual: &[JcrRecord], expected: &[JcrRecord]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert_eq!(
            (
                &a.title,
                &a.issn,
                &a.eissn,
                a.year,
                &a.category,
                &a.quartile,
                a.jif,
                a.row
            ),
            (
                &b.title,
                &b.issn,
                &b.eissn,
                b.year,
                &b.category,
                &b.quartile,
                b.jif,
                b.row
            )
        );
    }
}

fn assert_import(actual: &JcrImport, expected: &JcrImport) {
    assert_eq!(actual.file_name, expected.file_name);
    assert_eq!(actual.file_sha256, expected.file_sha256);
    assert_eq!(actual.imported_at, expected.imported_at);
    assert_eq!(actual.warnings, expected.warnings);
    assert_records(&actual.records, &expected.records);
}

fn expected_records() -> Vec<JcrRecord> {
    let record =
        |title: &str, issn: &str, eissn: Option<&str>, year, category: &str, quartile, jif, row| {
            JcrRecord {
                title: title.into(),
                issn: Some(issn.into()),
                eissn: eissn.map(str::to_owned),
                year,
                category: category.into(),
                quartile,
                jif,
                row,
            }
        };
    vec![
        record(
            "Journal of \"Cells\", Biology",
            "1234-5678",
            Some("8765-432X"),
            2023,
            "Oncology",
            Quartile::Q2,
            Some(2.5),
            1,
        ),
        record(
            "Journal of \"Cells\", Biology",
            "1234-5678",
            Some("8765-432X"),
            2024,
            "Oncology",
            Quartile::Q1,
            Some(3.5),
            3,
        ),
        record(
            "Journal of \"Cells\", Biology",
            "1234-5678",
            Some("8765-432X"),
            2024,
            "Hematology",
            Quartile::Q3,
            None,
            4,
        ),
        record(
            "Multiline\nJournal",
            "1111-2222",
            None,
            2022,
            "Public Health, General",
            Quartile::Q4,
            Some(1.25),
            5,
        ),
    ]
}

// A minimal ZIP writer: both local headers and central directory describe each entry.
fn u16le(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn u32le(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn zip(entries: &[(&str, &[u8], u16, u16)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for &(name, data, method, flags) in entries {
        let compressed = if method == 8 {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let mut crc = Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let offset = out.len() as u32;
        let size = compressed.len() as u32;
        let plain = data.len() as u32;
        let name_len = name.len() as u16;
        u32le(&mut out, 0x04034b50);
        u16le(&mut out, 20);
        u16le(&mut out, flags);
        u16le(&mut out, method);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, plain);
        u16le(&mut out, name_len);
        u16le(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&compressed);
        u32le(&mut central, 0x02014b50);
        u16le(&mut central, 20);
        u16le(&mut central, 20);
        u16le(&mut central, flags);
        u16le(&mut central, method);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u32le(&mut central, crc);
        u32le(&mut central, size);
        u32le(&mut central, plain);
        u16le(&mut central, name_len);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let offset = out.len() as u32;
    let size = central.len() as u32;
    out.extend_from_slice(&central);
    u32le(&mut out, 0x06054b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, entries.len() as u16);
    u16le(&mut out, entries.len() as u16);
    u32le(&mut out, size);
    u32le(&mut out, offset);
    u16le(&mut out, 0);
    out
}

fn workbook() -> Vec<u8> {
    let content_types = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#;
    let workbook = br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="JCR" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let shared = br#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Journal of "Cells", Biology</t></si><si><t>1234-5678</t></si><si><t>8765-432X</t></si><si><t>Oncology</t></si><si><t>Hematology</t></si></sst>"#;
    let sheet = br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Journal name</t></is></c><c r="B1" t="inlineStr"><is><t>ISSN</t></is></c><c r="C1" t="inlineStr"><is><t>eISSN</t></is></c><c r="D1" t="inlineStr"><is><t>Category</t></is></c><c r="E1" t="inlineStr"><is><t>Year</t></is></c><c r="F1" t="inlineStr"><is><t>JIF Quartile</t></is></c><c r="G1" t="inlineStr"><is><t>Journal Impact Factor</t></is></c></row>
<row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2" t="s"><v>1</v></c><c r="C2" t="s"><v>2</v></c><c r="D2" t="s"><v>3</v></c><c r="E2"><v>2023</v></c><c r="F2" t="inlineStr"><is><t>Q2</t></is></c><c r="G2"><v>2.5</v></c></row>
<row r="3"><c r="A3" t="s"><v>0</v></c><c r="B3" t="s"><v>1</v></c><c r="C3" t="s"><v>2</v></c><c r="D3" t="s"><v>3</v></c><c r="E3"><v>2023</v></c><c r="F3" t="inlineStr"><is><t>Q4</t></is></c><c r="G3"><v>9.9</v></c></row>
<row r="4"><c r="A4" t="s"><v>0</v></c><c r="B4" t="s"><v>1</v></c><c r="C4" t="s"><v>2</v></c><c r="D4" t="s"><v>3</v></c><c r="E4"><v>2024</v></c><c r="F4" t="inlineStr"><is><t>Q1</t></is></c><c r="G4"><v>3.5</v></c></row>
<row r="5"><c r="A5" t="s"><v>0</v></c><c r="B5" t="s"><v>1</v></c><c r="C5" t="s"><v>2</v></c><c r="D5" t="s"><v>4</v></c><c r="E5"><v>2024</v></c><c r="F5" t="inlineStr"><is><t>Q3</t></is></c></row>
<row r="6"><c r="A6" t="inlineStr"><is><t>Multiline
Journal</t></is></c><c r="B6" t="inlineStr"><is><t>1111-2222</t></is></c><c r="D6" t="inlineStr"><is><t>Public Health, General</t></is></c><c r="E6"><v>2022</v></c><c r="F6" t="inlineStr"><is><t>Q4</t></is></c><c r="G6"><v>1.25</v></c></row>
</sheetData></worksheet>"#;
    zip(&[
        ("[Content_Types].xml", content_types, 0, 0),
        ("xl/workbook.xml", workbook, 0, 0),
        ("xl/sharedStrings.xml", shared, 0, 0),
        ("xl/worksheets/sheet1.xml", sheet, 8, 0),
    ])
}

#[test]
fn csv_import_records_hash_duplicates_and_quoting() {
    let path = fixture("journals.csv");
    let import = jcr::import(&path).unwrap();
    assert_eq!(import.file_name, "journals.csv");
    assert_eq!(import.file_sha256, sha256_hex(&fs::read(path).unwrap()));
    assert!(!import.imported_at.is_empty());
    assert_records(&import.records, &expected_records());
    assert_eq!(import.warnings.len(), 1);
    assert_eq!(import.records[0].quartile, Quartile::Q2); // duplicate Q4 must not win
}

#[test]
fn semicolon_csv_has_the_same_records() {
    let result = jcr::import(&fixture("semicolon.csv")).unwrap();
    assert_records(&result.records, &expected_records());
    assert_eq!(result.warnings.len(), 1);
}

#[test]
fn tab_delimiter_and_optional_jif() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("tab.csv");
    fs::write(&path, "Journal name\tISSN\teISSN\tCategory\tYear\tJIF Quartile\nExample\t1234-5678\t8765-432X\tOncology\t2024\tQ1\n").unwrap();
    let result = jcr::import(&path).unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].jif, None);
    assert_eq!(result.records[0].quartile, Quartile::Q1);
}
#[test]
fn xlsx_import_matches_csv() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("journals.xlsx");
    fs::write(&path, workbook()).unwrap();
    let csv = jcr::import(&fixture("journals.csv")).unwrap();
    let xlsx = jcr::import(&path).unwrap();
    assert_records(&xlsx.records, &csv.records);
    assert_eq!(xlsx.warnings.len(), 1);
    assert_eq!(xlsx.file_sha256, sha256_hex(&fs::read(path).unwrap()));
}

#[test]
fn missing_column_and_unknown_extension_report_kinds() {
    let error = match jcr::import(&fixture("missing_quartile.csv")) {
        Err(e) => e,
        Ok(_) => panic!("missing quartile accepted"),
    };
    assert_eq!(error.kind, "jcr_columns");
    assert!(
        error.message.to_lowercase().contains("quartile"),
        "{}",
        error.message
    );
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("journals.txt");
    fs::write(&path, b"irrelevant").unwrap();
    let error = match jcr::import(&path) {
        Err(e) => e,
        Ok(_) => panic!("unknown extension accepted"),
    };
    assert_eq!(error.kind, "jcr_format");
}

#[test]
fn quartiles_use_matching_issns_and_requested_or_newest_year() {
    let import = jcr::import(&fixture("journals.csv")).unwrap();
    let latest = import.quartiles_for(&["8765432x".into()], None);
    assert_eq!(latest.len(), 2);
    for (category, quartile, row) in [
        ("Oncology", Quartile::Q1, 3),
        ("Hematology", Quartile::Q3, 4),
    ] {
        let value = latest
            .iter()
            .find(|q| q.value.category == category)
            .unwrap();
        assert_eq!(value.value.quartile, quartile);
        assert_eq!(value.value.scheme, QuartileScheme::Jcr);
        assert_eq!(value.value.year, 2024);
        assert_eq!(
            value.value.label(),
            format!(
                "JCR {} ({category}, 2024)",
                if row == 3 { "Q1" } else { "Q3" }
            )
        );
        assert_eq!(value.source.kind, SourceKind::JcrImport);
        assert_eq!(value.source.url, None);
        assert_eq!(value.source.fetched_at, import.imported_at);
        assert_eq!(value.source.data_year, Some(2024));
        assert_eq!(
            value.source.raw,
            format!("{}:{row}:quartile", import.file_sha256)
        );
    }
    let old = import.quartiles_for(&["12345678".into()], Some(2023));
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].value.quartile, Quartile::Q2);
    assert_eq!(old[0].value.label(), "JCR Q2 (Oncology, 2023)");
    assert_eq!(old[0].source.data_year, Some(2023));
    assert_eq!(
        old[0].source.raw,
        format!("{}:1:quartile", import.file_sha256)
    );
    assert!(import
        .quartiles_for(&["12345678".into()], Some(2022))
        .is_empty());
    assert!(import.quartiles_for(&["00000000".into()], None).is_empty());
}

#[test]
fn save_and_load_latest_round_trip() {
    let temp = TempDir::new().unwrap();
    assert!(jcr::load_latest(temp.path()).unwrap().is_none());
    let mut earlier = jcr::import(&fixture("journals.csv")).unwrap();
    let mut later = jcr::import(&fixture("semicolon.csv")).unwrap();
    earlier.imported_at = "2024-01-01T00:00:00Z".into();
    later.imported_at = "2025-01-01T00:00:00Z".into();
    jcr::save(temp.path(), &later).unwrap();
    jcr::save(temp.path(), &earlier).unwrap(); // save order must not determine latest
    for import in [&earlier, &later] {
        assert!(model::data_dir(temp.path())
            .join("jcr")
            .join(format!("{}.json", import.file_sha256))
            .is_file());
    }
    let loaded = jcr::load_latest(temp.path()).unwrap().unwrap();
    assert_import(&loaded, &later);
}

#[test]
fn jcr_values_can_be_appended_without_changing_sjr() {
    let sjr = Sourced {
        value: QuartileValue {
            quartile: Quartile::Q1,
            scheme: QuartileScheme::Sjr,
            category: "Oncology".into(),
            year: 2024,
        },
        source: SourceTag {
            kind: SourceKind::Sjr,
            url: Some("sjr-source".into()),
            fetched_at: "2025-01-01T00:00:00Z".into(),
            data_year: Some(2024),
            raw: "sjr-raw".into(),
        },
    };
    let mut values = vec![sjr];
    let original = values.clone();
    let import = jcr::import(&fixture("journals.csv")).unwrap();
    values.extend(import.quartiles_for(&["1234-5678".into()], None));
    assert_eq!(&values[..original.len()], original.as_slice());
    let oncology = values
        .iter()
        .find(|q| q.value.scheme == QuartileScheme::Jcr && q.value.category == "Oncology")
        .unwrap();
    assert_eq!(values[0].value.label(), "SJR Q1 (Oncology, 2024)");
    assert_eq!(oncology.value.label(), "JCR Q1 (Oncology, 2024)");
}

#[test]
fn xlsx_rejects_truncation_garbage_encryption_and_zip64() {
    let valid = workbook();
    let encrypted = zip(&[("xl/workbook.xml", b"encrypted", 0, 1)]);
    let mut zip64 = valid.clone();
    let eocd = zip64.len() - 22;
    zip64[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    for bytes in [
        &valid[..valid.len() - 10],
        &b"garbage"[..],
        &encrypted,
        &zip64,
    ] {
        let error = match xlsx::read_first_sheet(bytes) {
            Err(e) => e,
            Ok(_) => panic!("invalid XLSX accepted"),
        };
        assert_eq!(error.kind, "xlsx");
    }
}

#[test]
fn jcr_sources_do_not_use_network_interfaces() {
    for name in ["jcr.rs", "xlsx.rs"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/paper/journals")
            .join(name);
        let source = fs::read_to_string(path).unwrap();
        for forbidden in ["Transport", "curl", "http"] {
            assert!(!source.contains(forbidden), "{name} contains {forbidden}");
        }
    }
    let _: fn(&Path) -> axiom::base::Res<JcrImport> = jcr::import;
}
