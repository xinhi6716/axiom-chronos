use axiom::orch::{
    anchor_text, contract_block, dispatcher_rules, Anchor, CONTRACT_BUDGET, RESTORE_CAP,
    RULES_PATH, WISH_BUDGET,
};
use std::{fs, path::Path};

#[test]
fn dispatcher_matches_embedded_body() {
    let raw = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == RULES_PATH)
        .unwrap()
        .1
        .replace("\r\n", "\n");
    let expected = raw
        .split_inclusive('\n')
        .skip_while(|line| *line != "---\n")
        .skip(1)
        .skip_while(|line| *line != "---\n")
        .skip(1)
        .collect::<String>();
    assert_eq!(dispatcher_rules(), expected.trim_start_matches('\n'));
    assert!(!dispatcher_rules().starts_with("---"));
    assert!(!dispatcher_rules().contains("\r\n"));
}

#[test]
fn contract_preserves_text_and_isolates_modules() {
    let map = "## m1 \r\nnotes\r\nCONTRACTISH no\r\nCONTRACT  原文  \r\n  中文  字\t\r\n\tinner  \r\n\r\n \t\r\n## m2\r\nCONTRACT second\r\n  secret\r\n";
    assert_eq!(
        contract_block(map, "m1").as_deref(),
        Some("CONTRACT  原文  \n  中文  字\t\n\tinner  ")
    );
    assert_eq!(
        contract_block(map, "m2").as_deref(),
        Some("CONTRACT second\n  secret")
    );
    assert!(!contract_block(map, "m1").unwrap().contains("secret"));
    assert_eq!(contract_block(map, "missing"), None);
    assert_eq!(
        contract_block("## m1\nCONTRACTISH no\n## m2\nCONTRACT yes", "m1"),
        None
    );
    assert_eq!(
        contract_block("## m1\nCONTRACT\n  kept\nunindented\n  excluded", "m1").as_deref(),
        Some("CONTRACT\n  kept")
    );
    assert_eq!(
        contract_block("## m1\nCONTRACT\n\n  after blank\n", "m1").as_deref(),
        Some("CONTRACT\n\n  after blank")
    );
    assert_eq!(
        contract_block("## m1\r\nCONTRACT\r\r\n  raw\r\r\nend", "m1").as_deref(),
        Some("CONTRACT\r\n  raw\r")
    );
    for end in ["## m2", "OUT OF SCOPE details", "FULL TEST details"] {
        let text = format!("## m1\nCONTRACT good\n  kept\n{end}\nCONTRACT bad\n  excluded");
        assert_eq!(
            contract_block(&text, "m1").as_deref(),
            Some("CONTRACT good\n  kept")
        );
        let absent = format!("## m1\n{end}\nCONTRACT bad");
        assert_eq!(contract_block(&absent, "m1"), None);
    }
    assert_eq!(
        contract_block("## m1\nCONTRACT first\n## m1\nCONTRACT second", "m1").as_deref(),
        Some("CONTRACT first")
    );
}

#[test]
fn anchors_respect_order_budgets_cap_and_store() {
    let store = Path::new(env!("CARGO_MANIFEST_DIR")).join(".axiom-tmp/restore1");
    fs::create_dir_all(&store).unwrap();
    let map_path = store.join("modules.md");
    let map_display = map_path.display();
    let missing = Anchor {
        rules: false,
        wish: None,
        wish_file: None,
        module: Some("absent"),
    };
    assert_eq!(
        anchor_text(&store, &missing),
        format!("CONTRACT of module absent: not found in {map_display}.\n")
    );
    assert_eq!(
        anchor_text(
            &store,
            &Anchor {
                module: None,
                ..missing
            }
        ),
        ""
    );

    let map = "## m1\r\nCONTRACT 甲\r\n  keep\r\n## m2\r\nCONTRACT secret-m2\r\n";
    fs::write(&map_path, map).unwrap();
    let a = Anchor {
        rules: true,
        wish: Some("短 wish"),
        wish_file: None,
        module: Some("m1"),
    };
    let expected = format!("BINDING RULES, verbatim from agents/dispatcher.md:\n{}\nWISH, verbatim:\n短 wish\nCONTRACT of module m1, verbatim from {map_display}:\nCONTRACT 甲\n  keep\n", dispatcher_rules());
    let actual = anchor_text(&store, &a);
    assert_eq!(actual, expected);
    assert!(!actual.contains("secret-m2"));
    assert!(!actual.contains("INCOMPLETE:"));
    assert_eq!(actual, anchor_text(&store, &a));
    assert_eq!(fs::read_to_string(&map_path).unwrap(), map);
    assert_eq!(fs::read_dir(&store).unwrap().count(), 1);
    assert_eq!(
        anchor_text(&store, &missing),
        format!("CONTRACT of module absent: not found in {map_display}.\n")
    );

    let wish = format!("{}界more", "a".repeat(WISH_BUDGET - 1));
    let block = format!(
        "CONTRACT {}界more",
        "b".repeat(CONTRACT_BUDGET - "CONTRACT ".len() - 1)
    );
    let long_map = format!("## m1\n{block}\n## m2\nCONTRACT not-m1\n");
    fs::write(&map_path, &long_map).unwrap();
    let wish_file = Path::new("goal.md");
    let long = anchor_text(
        &store,
        &Anchor {
            rules: false,
            wish: Some(&wish),
            wish_file: Some(wish_file),
            module: Some("m1"),
        },
    );
    assert_eq!(long, format!("WISH, verbatim:\n{}\nINCOMPLETE: wish is cut at {} of {} bytes; read the rest in {}.\nCONTRACT of module m1, verbatim from {map_display}:\n{}\nINCOMPLETE: contract is cut at {} of {} bytes; read the rest in {map_display}.\n", &wish[..WISH_BUDGET-1], WISH_BUDGET-1, wish.len(), wish_file.display(), &block[..CONTRACT_BUDGET-1], CONTRACT_BUDGET-1, block.len()));
    assert!(!long.contains("not-m1"));
    let no_file = anchor_text(
        &store,
        &Anchor {
            rules: false,
            wish: Some(&wish),
            wish_file: None,
            module: None,
        },
    );
    assert!(no_file.contains("read the rest in the goal file.\n"));

    let exact_block = format!(
        "CONTRACT {}",
        "c".repeat(CONTRACT_BUDGET - "CONTRACT ".len())
    );
    fs::write(&map_path, format!("## m1\n{exact_block}\n")).unwrap();
    let exact_wish = "w".repeat(WISH_BUDGET);
    let full = anchor_text(
        &store,
        &Anchor {
            rules: true,
            wish: Some(&exact_wish),
            wish_file: None,
            module: Some("m1"),
        },
    );
    assert!(!full.contains("INCOMPLETE:"));
    assert!(full.contains(&dispatcher_rules()));
    assert!(full.contains(&exact_block));
    assert!(full.contains(&exact_wish));
    assert!(full.len() <= RESTORE_CAP);

    let huge_module = "界".repeat(5000);
    let cut = anchor_text(
        &store,
        &Anchor {
            rules: true,
            wish: Some(&exact_wish),
            wish_file: None,
            module: Some(&huge_module),
        },
    );
    assert!(cut.len() <= RESTORE_CAP);
    let prefix = "BINDING RULES, verbatim from agents/dispatcher.md:\n";
    let marker = "\nINCOMPLETE: rules is cut at ";
    let (kept, tail) = cut
        .strip_prefix(prefix)
        .unwrap()
        .split_once(marker)
        .unwrap();
    assert!(dispatcher_rules().starts_with(kept));
    assert!(tail.starts_with(&format!(
        "{} of {} bytes; read the rest in ${{CLAUDE_PLUGIN_ROOT}}/agents/dispatcher.md.\n",
        kept.len(),
        dispatcher_rules().len()
    )));
    assert!(cut.contains(&format!(
        "CONTRACT of module {huge_module}: not found in {map_display}.\n"
    )));
    assert_eq!(
        cut,
        anchor_text(
            &store,
            &Anchor {
                rules: true,
                wish: Some(&exact_wish),
                wish_file: None,
                module: Some(&huge_module)
            }
        )
    );
    let enormous_module = "界".repeat(RESTORE_CAP);
    let enormous = anchor_text(
        &store,
        &Anchor {
            rules: false,
            wish: None,
            wish_file: None,
            module: Some(&enormous_module),
        },
    );
    assert!(enormous.len() <= RESTORE_CAP);
    assert_eq!(
        fs::read_to_string(&map_path).unwrap(),
        format!("## m1\n{exact_block}\n")
    );
    fs::remove_file(&map_path).unwrap();
    fs::remove_dir(&store).unwrap();
}

#[test]
fn oversized_module_keeps_contract_incomplete_line_whole() {
    let store = Path::new(env!("CARGO_MANIFEST_DIR")).join(".axiom-tmp/restore-long-module");
    fs::create_dir_all(&store).unwrap();
    let module = "界".repeat(10_000);
    let contract = format!("CONTRACT {}", "x".repeat(CONTRACT_BUDGET));
    let map_path = store.join("modules.md");
    fs::write(&map_path, format!("## {module}\n{contract}\n")).unwrap();

    let output = anchor_text(
        &store,
        &Anchor {
            rules: false,
            wish: None,
            wish_file: None,
            module: Some(&module),
        },
    );
    assert!(output.len() <= RESTORE_CAP);
    let last_line = output.split_inclusive('\n').last().unwrap();
    assert_eq!(
        last_line,
        format!(
            "INCOMPLETE: contract is cut at {CONTRACT_BUDGET} of {} bytes; read the rest in {}.\n",
            contract.len(),
            map_path.display()
        )
    );

    let wish = "w".repeat(WISH_BUDGET + 1);
    let output_with_wish = anchor_text(
        &store,
        &Anchor {
            rules: false,
            wish: Some(&wish),
            wish_file: None,
            module: Some(&module),
        },
    );
    assert!(output_with_wish.len() <= RESTORE_CAP);
    let markers: Vec<_> = output_with_wish
        .split_inclusive('\n')
        .filter(|line| line.starts_with("INCOMPLETE:"))
        .collect();
    assert_eq!(markers.len(), 2);
    assert_eq!(
        markers[0],
        format!("INCOMPLETE: wish is cut at {WISH_BUDGET} of {} bytes; read the rest in the goal file.\n", wish.len())
    );
    assert_eq!(markers[1], last_line);

    fs::remove_file(map_path).unwrap();
    fs::remove_dir(store).unwrap();
}
