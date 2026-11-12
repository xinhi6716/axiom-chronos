use axiom::engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const REFERENCES: &[&str] = &[
    "AIScientists-Dev/academic-humanizer",
    "Imbad0202/academic-research-skills",
    "jin-s13/ai-research-writing-skill",
    "appautomaton/latex-arxiv-SKILL",
    "yunshenwuchuxun/latex-paper-skills",
    "bahayonghang/academic-writing-skills",
    "Zhangyanbo/vibe-paper-writing",
    "Rezenders/scientific_writing_agent",
    "K-Dense-AI/scientific-agent-skills",
    "Plusero/agentic-research-skills",
    "y9c/scifig",
    "giacolees/scifig",
    "yanggaihao5-sys/dsh-sci-figures",
    "ACG-j/academic-skills",
    "WUWeifeng710/make-my-figure-skill",
    "vladimirc777/journal-finder",
    "ikhwan-arief/journal-indexer-scientific-discovery",
    "nature-figure",
    "nature-writing",
    "nature-polishing",
    "nature-statistics",
    "nature-citation",
    "nature-ref-verifier",
    "nature-academic-search",
    "nature-paper2word",
    "nature-response",
    "nature-reviewer",
    "nature-data",
];
const FIELDS: &[&str] = &["Source", "Version", "Licence", "Decision", "Reason"];
const FAILED: &str = "not evaluated: fetch failed 2026-11-13";

fn check_doc(text: &str) -> Result<BTreeMap<&str, BTreeMap<&str, &str>>, String> {
    let mut sections = BTreeMap::<&str, BTreeMap<&str, &str>>::new();
    let mut current = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            if sections.insert(name, BTreeMap::new()).is_some() {
                return Err(format!("duplicate section {name}"));
            }
            current = Some(name);
        } else if let Some(name) = current {
            for field in FIELDS {
                if let Some(value) = line.strip_prefix(&format!("{field}:")) {
                    let value = value.trim();
                    if value.is_empty()
                        || sections
                            .get_mut(name)
                            .unwrap()
                            .insert(field, value)
                            .is_some()
                    {
                        return Err(format!("empty or duplicate {field} in {name}"));
                    }
                }
            }
        }
    }
    let actual: BTreeSet<_> = sections.keys().copied().collect();
    let expected: BTreeSet<_> = REFERENCES.iter().copied().collect();
    if actual != expected {
        return Err("section set differs".into());
    }
    for (&name, fields) in &sections {
        if fields.len() != FIELDS.len() {
            return Err(format!("missing field in {name}"));
        }
        let decision = fields["Decision"];
        if !["adopt", "adapt", "reject", "reference-only", FAILED].contains(&decision) {
            return Err(format!("invalid decision in {name}"));
        }
        let version = fields["Version"];
        if name.starts_with("nature-") {
            if version != "local (undated)"
                || fields["Licence"] != "local personal skill"
                || decision != "reference-only"
            {
                return Err(format!("invalid local entry {name}"));
            }
        } else if name == "vladimirc777/journal-finder" {
            if decision != FAILED || version != "none" || fields["Licence"] != "unknown" {
                return Err("invalid failed fetch".into());
            }
        } else {
            if decision == FAILED
                || version.len() != 40
                || !version.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(format!("invalid version or fetch decision in {name}"));
            }
            if (fields["Licence"].starts_with("none found")
                || fields["Licence"].starts_with("CC")
                || fields["Licence"].starts_with("PolyForm")
                || fields["Licence"] == "unknown")
                && !["reference-only", "reject"].contains(&decision)
            {
                return Err(format!("invalid licence decision in {name}"));
            }
        }
    }
    let humanizer = &sections["AIScientists-Dev/academic-humanizer"];
    if humanizer["Decision"] != "adopt" || humanizer["Licence"] != "MIT" {
        return Err("invalid humanizer decision".into());
    }
    Ok(sections)
}

fn check_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(format!("hash mismatch: {actual}"))
    }
}

#[test]
fn reference_decisions_and_negative_controls() {
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/paper-adoption.md"))
            .unwrap();
    check_doc(&doc).unwrap();
    let start = doc.find("### nature-figure\n").unwrap();
    let end = start + doc[start + 4..].find("### ").unwrap() + 4;
    let without_section = format!("{}{}", &doc[..start], &doc[end..]);
    assert!(check_doc(&without_section).is_err());
    assert!(check_doc(&doc.replacen("Decision: adopt", "Decision: maybe", 1)).is_err());
    let local = &doc[start..end];
    let bad_local = format!(
        "{}{}{}",
        &doc[..start],
        local.replacen("Decision: reference-only", "Decision: adopt", 1),
        &doc[end..]
    );
    assert!(check_doc(&bad_local).is_err());
}

#[test]
fn vendored_asset_integrity_and_negative_control() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = root.join("engine/paper/academic-humanizer");
    let source: Value =
        serde_json::from_slice(&fs::read(base.join("SOURCE.json")).unwrap()).unwrap();
    let keys: BTreeSet<_> = source
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "url",
            "commit",
            "version",
            "sha256_skill",
            "sha256_license",
            "fetched_at",
            "adaptations"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        source["url"],
        "https://github.com/AIScientists-Dev/academic-humanizer"
    );
    assert_eq!(source["commit"], "94b88b23703bed7df507acae7d6d5876209a0cdf");
    assert_eq!(source["version"], "0.3.3");
    assert_eq!(source["fetched_at"], "2026-11-13");
    assert_eq!(source["adaptations"], serde_json::json!([]));
    let skill = fs::read(base.join("SKILL.md")).unwrap();
    let license = fs::read(base.join("LICENSE")).unwrap();
    check_hash(&skill, source["sha256_skill"].as_str().unwrap()).unwrap();
    check_hash(&license, source["sha256_license"].as_str().unwrap()).unwrap();
    let mut changed = skill.clone();
    changed[0] ^= 1;
    assert!(check_hash(&changed, source["sha256_skill"].as_str().unwrap()).is_err());
    let license_text = std::str::from_utf8(&license).unwrap();
    assert!(
        license_text.contains("MIT License")
            && license_text.contains("Permission is hereby granted")
    );
    let skill_text = std::str::from_utf8(&skill).unwrap();
    let frontmatter = skill_text
        .strip_prefix("---\n")
        .unwrap()
        .split("\n---")
        .next()
        .unwrap();
    let version = frontmatter
        .lines()
        .find_map(|line| line.strip_prefix("version: "))
        .unwrap();
    assert_eq!(version, source["version"].as_str().unwrap());
    for file in ["SKILL.md", "LICENSE", "SOURCE.json"] {
        let path = format!("paper/academic-humanizer/{file}");
        assert_eq!(
            engine::asset(&path).unwrap().as_bytes(),
            fs::read(base.join(file)).unwrap()
        );
    }
    let doc = fs::read_to_string(root.join("docs/paper-adoption.md")).unwrap();
    let sections = check_doc(&doc).unwrap();
    assert_eq!(
        sections["AIScientists-Dev/academic-humanizer"]["Version"],
        source["commit"].as_str().unwrap()
    );
}
