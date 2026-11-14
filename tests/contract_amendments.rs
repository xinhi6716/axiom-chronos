//! A module's contract carries every AMENDMENT block the map adds to it.
use axiom::orch::{contract_block, module_context, MAX_CONTEXT_BYTES};

const MAP: &str = "## m1
PURPOSE one
PATHS src/m1/
DEPENDS none
CONTRACT   first rule
             detail
AMENDMENT  C17 (new unit):
             placement rule

             after a blank line
ACCEPT     tests/m1.rs `cargo test`
AMENDMENT  R4 follow-up:
             units in this order
SERVES     C1
OUT OF SCOPE C9 not built
AMENDMENT  R5 after out of scope
  still R5
CONTRACT   second contract, ignored
  ignored too
FULL TEST  `cargo test`
AMENDMENT  R6 after full test
AMENDMENT
  bare heading
AMENDMENTS not a field
## m2
PURPOSE two
PATHS src/m2/
DEPENDS m1
CONTRACT   other module
AMENDMENT  M2 belongs to m2
ACCEPT     tests/m2.rs `cargo test`
";

#[test]
fn every_amendment_block_of_the_module_is_kept_in_map_order() {
    assert_eq!(
        contract_block(MAP, "m1").as_deref(),
        Some(
            "CONTRACT   first rule
             detail
AMENDMENT  C17 (new unit):
             placement rule

             after a blank line
AMENDMENT  R4 follow-up:
             units in this order
AMENDMENT  R5 after out of scope
  still R5
AMENDMENT  R6 after full test
AMENDMENT
  bare heading"
        )
    );
    assert_eq!(
        contract_block(MAP, "m2").as_deref(),
        Some("CONTRACT   other module\nAMENDMENT  M2 belongs to m2")
    );
    assert_eq!(
        contract_block(
            "## m1\r\nCONTRACT a\r\nACCEPT x\r\nAMENDMENT  B\r\n  b\r\n",
            "m1"
        )
        .as_deref(),
        Some("CONTRACT a\nAMENDMENT  B\n  b")
    );
}

#[test]
fn module_context_carries_amendments_and_marks_a_cut() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path();
    let units = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
    std::fs::write(store.join("units.tsv"), units).unwrap();
    let small = MAP.split("FULL TEST").next().unwrap();
    std::fs::write(store.join("modules.md"), small).unwrap();
    let c = module_context(store, "m1", store).unwrap();
    assert!(c.contract.contains("AMENDMENT  C17"), "{}", c.contract);
    assert!(c.contract.contains("AMENDMENT  R5"), "{}", c.contract);
    assert!(!c.incomplete.iter().any(|i| i.section == "contract"));

    let long = format!("             {}\n", "x".repeat(200)).repeat(MAX_CONTEXT_BYTES / 200);
    let map = format!(
        "## m1\nPURPOSE one\nPATHS src/m1/\nDEPENDS none\nCONTRACT   rule\nACCEPT     tests/m1.rs `cargo test`\nAMENDMENT  BIG\n{long}"
    );
    std::fs::write(store.join("modules.md"), map).unwrap();
    let c = module_context(store, "m1", store).unwrap();
    assert!(c.contract.contains("AMENDMENT  BIG"), "{}", c.contract);
    assert!(
        c.incomplete.iter().any(|i| i.section == "contract"),
        "a cut amendment must mark the contract incomplete"
    );
}
