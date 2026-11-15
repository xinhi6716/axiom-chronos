use axiom::{base::schema::validate, paper::*};
use serde_json::{json, Value};

fn schema(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("engine/paper/{name}.schema.json")).unwrap()).unwrap()
}
fn sample() -> Value {
    json!({"title":"Study","abstract":{"id":"a","text":"Result {{q:q1}} [@ref]","claims":[{"claim":"c1","evidence":["E1"]}]},"keywords":["test"],"sections":[{"id":"s1","heading":"Results","paragraphs":[{"id":"p1","text":"{{q:q2}} [@z; @a]","claims":[]}]}],"figures":[{"id":"f1","kind":"line","artifact":"data.csv","x":"x","y":["y"],"error":"sd","preset":"default","caption":{"id":"fc","text":"{{fig:f1}}","claims":[]}}],"tables":[{"id":"t1","artifact":"data.csv","columns":[{"column":"x","header":"X","precision":2}],"caption":{"id":"tc","text":"{{tab:t1}} {{sec:s1}}","claims":[]}}]})
}
#[test]
fn schemas_strict_and_valid_json() {
    fn check(v: &Value) {
        if v.get("type") == Some(&json!("object")) {
            assert_eq!(v["additionalProperties"], false);
            let props = v["properties"].as_object().unwrap();
            let required = v["required"].as_array().unwrap();
            assert_eq!(props.len(), required.len());
            for (key, child) in props { assert!(required.contains(&json!(key))); check(child); }
        }
        if let Some(items) = v.get("items") { check(items); }
    }
    check(&schema("pass")); check(&schema("reviewer"));
    let reviewer = json!({"verdict":"revise","findings":[{"location":"x","severity":"major","category":"citation","issue":"x","evidence":"x"}],"checked":["citations"]});
    assert!(validate(&schema("reviewer"), &reviewer).is_ok());
    let mut invalid = reviewer; invalid["findings"][0]["extra"] = json!(1);
    assert!(validate(&schema("reviewer"), &invalid).is_err());
}
#[test]
fn pass_schema_accepts_sample_and_rejects_extra_field() {
    let mut pass = json!({"manuscript":sample(),"change_log":[{"location":"abstract","kind":"edited","reason":"clarity"}]});
    let s = schema("pass"); assert!(validate(&s,&pass).is_ok());
    pass["manuscript"]["sections"][0]["paragraphs"][0]["extra"] = json!(true);
    assert!(validate(&s,&pass).is_err());
}
#[test]
fn manuscript_member_json_roundtrip() {
    let raw = sample(); let manuscript = Manuscript::from_member_json(&raw).unwrap();
    assert_eq!(manuscript.to_member_json(), raw);
    assert_eq!(manuscript.hash(), Manuscript::from_member_json(&raw).unwrap().hash());
    assert_eq!(Manuscript::from_member_json(&json!({})).unwrap_err().kind, "malformed");
}
#[test]
fn token_grammar_parses_all_forms_and_ignores_plain_text() {
    let text = "é {{q:a.1}} {{fig:f-1}} {{tab:t:2}} {{sec:s_3}} [@k] [@k1; @k2] {{q:bad id}} [@bad key]";
    let found = tokens(text);
    assert_eq!(found.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(), vec![Token::Quantity("a.1".into()),Token::Fig("f-1".into()),Token::Tab("t:2".into()),Token::Sec("s_3".into()),Token::Cite(vec!["k".into()]),Token::Cite(vec!["k1".into(),"k2".into()])]);
    for (range, _) in found { assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end)); }
    assert!(tokens("plain text and {{q:}} [@]").is_empty());
}
#[test]
fn invariants_keyed_by_location() {
    let m = Manuscript::from_member_json(&sample()).unwrap(); let inv = m.invariants();
    assert_eq!(m.paragraphs().iter().map(|(p,_)| p.as_str()).collect::<Vec<_>>(), vec!["abstract","sec:s1/p:p1","fig:f1/caption","tab:t1/caption"]);
    assert_eq!(inv.claims["abstract"][0].claim, "c1");
    assert_eq!(inv.quantities["abstract"], vec!["q1"]);
    assert_eq!(inv.citations["sec:s1/p:p1"], vec!["a","z"]);
}
#[test]
fn known_unknown_has_no_zero() {
    let unknown: Known<u32> = Known::Unknown { reason: "not published".into() };
    assert_eq!(unknown.value(), None);
    let known = Known::Known(Sourced { value: 0u32, source: SourceTag { kind: SourceKind::Doaj, url: None, fetched_at: "now".into(), data_year: None, raw: "x".into() } });
    assert_eq!(known.value(), Some(&0));
}
#[test]
fn quartile_labels_name_scheme() {
    let mut q = QuartileValue { quartile: Quartile::Q1, scheme: QuartileScheme::Sjr, category: "Physics".into(), year: 2024 };
    assert_eq!(q.label(), "SJR Q1 (Physics, 2024)"); q.scheme = QuartileScheme::Jcr; q.quartile = Quartile::Q4;
    assert_eq!(q.label(), "JCR Q4 (Physics, 2024)");
}
#[test]
fn bundle_hash_excludes_hash_field() {
    let mut b = EvidenceBundle { version: 1, program: "p".into(), question: "q".into(), artifacts: vec![], claims: vec![], evidence: vec![], quantities: vec![], references: vec![], hash: "old".into() };
    let hash = b.compute_hash(); b.hash = "different".into(); assert_eq!(hash,b.compute_hash());
    let mut value = serde_json::to_value(&b).unwrap(); value.as_object_mut().unwrap().remove("hash");
    assert_eq!(hash, axiom::seams::sha256_hex(&serde_json::to_vec(&value).unwrap()));
    b.question = "changed".into(); assert_ne!(hash,b.compute_hash());
}
