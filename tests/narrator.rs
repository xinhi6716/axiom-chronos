use axiom::session::Renderer;
use serde_json::json;
use std::{fs, path::Path};

#[test]
fn narrator_asset_is_embedded_with_haiku_and_tools() {
    let disk =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/agents/narrator.md"))
            .unwrap();
    let embedded = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "agents/narrator.md")
        .unwrap()
        .1;
    assert_eq!(disk, embedded);
    let mut lines = disk.lines();
    assert_eq!(lines.next(), Some("---"));
    let frontmatter: Vec<_> = lines.take_while(|line| *line != "---").collect();
    assert!(frontmatter.contains(&"model: haiku"));
    assert!(frontmatter.contains(&"tools: Read, mcp__plugin_chronos_bridge__orch"));
    assert!(!disk.contains("decisions.md"));
    assert!(disk.contains("`<store>/astra/ruling.md` if present"));
    assert!(disk.contains("If the digest shows no units, report that ruling."));
}

#[test]
fn wish_starts_narrator_and_closes_goal_in_parallel() {
    let skill = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;
    let step_six = skill.lines().find(|line| line.starts_with("6. ")).unwrap();
    let step_seven = skill.lines().find(|line| line.starts_with("7. ")).unwrap();
    assert!(step_six.contains("two parallel calls"));
    assert!(step_six.contains("your whole reply"));
    assert_eq!(step_six.matches("Do not reprint").count(), 1);
    assert!(!step_six.contains("narrator's text"));
    assert!(step_seven.contains("same message as step 6's narrator call"));
    assert!(step_seven.contains("a `waiting` close has none"));
}

#[test]
fn wish_hands_the_digest_to_the_narrator() {
    let skill = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;
    let step = skill.lines().find(|line| line.starts_with("6. ")).unwrap();
    for phrase in [
        "chronos:narrator",
        r#"["report"]"#,
        "Do not reprint its text",
        "slug",
        "project directory",
        "store path",
    ] {
        assert!(step.contains(phrase), "missing {phrase}");
    }
}

#[test]
fn narrator_result_is_shown_but_other_agents_are_not() {
    for (agent, expected) in [
        (
            "chronos:narrator",
            "< chronos:narrator finished\n\nAll done.\n\n",
        ),
        ("worker", "< worker finished\n"),
    ] {
        let mut renderer = Renderer::new();
        let mut out = Vec::new();
        renderer.event(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"n","name":"Agent","input":{"subagent_type":agent}}]}}), &mut out);
        out.clear();
        renderer.event(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"n","content":"All done."}]}}), &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }
}
