use super::{github::*, modes::*, optimizer::*, templates::*};
use crate::desktop::{
    i18n::Lang,
    ux::seams::{FakeGh, GhOutcome, ModeKind, Proposal},
};
use std::{collections::HashSet, prelude::v1::test, time::Duration};

#[test]
fn templates_have_sections_and_unique_ids() {
    let all = templates();
    assert!(all.len() >= 4);
    assert_eq!(
        all.iter().map(|t| t.id).collect::<HashSet<_>>().len(),
        all.len()
    );
    for t in all {
        assert!(!t.title.0.is_empty() && !t.title.1.is_empty());
        for section in ["Goal", "Scope", "Limits", "Acceptance"] {
            assert!(t.body.0.contains(section));
        }
        for section in ["目標", "範圍", "限制", "驗收"] {
            assert!(t.body.1.contains(section));
        }
    }
}

#[test]
fn composition_and_diff_preserve_content() {
    let proposal = Proposal {
        goal: "goal".into(),
        scope: "scope".into(),
        limits: "limits".into(),
        acceptance: "acceptance".into(),
        ambiguities: vec!["question?".into()],
    };
    for lang in [Lang::En, Lang::ZhTw] {
        let result = compose("  Original\ntext\n", &proposal, &["answer".into()], lang);
        assert!(result.ends_with("  Original\ntext\n"));
        assert!(result.contains("answer") && result.contains("question?"));
    }
    assert_eq!(
        diff("same\nold", "same\nnew"),
        vec![
            (DiffKind::Same, "same".into()),
            (DiffKind::Removed, "old".into()),
            (DiffKind::Added, "new".into())
        ]
    );
}

#[test]
fn references_are_strict() {
    let expected = IssueRef {
        owner: "o".into(),
        repo: "r".into(),
        number: 42,
    };
    for value in [" o/r#42 ", "https://github.com/o/r/issues/42"] {
        assert_eq!(parse_reference(value), Ok(expected.clone()));
    }
    for value in [
        "junk",
        "o/r;rm#1",
        "o/r#1;rm",
        "o/r#0",
        "o/r/x#1",
        "https://github.com/o/r/issues/1/extra",
        "https://github.com/o/r/issues/1?x=y",
        "https://evil.com/o/r/issues/1",
    ] {
        assert_eq!(
            parse_reference(value),
            Err(ImportError::InvalidReference),
            "{value}"
        );
    }
}

#[test]
fn importing_uses_typed_arguments_and_maps_outcomes() {
    let r = parse_reference("o/r#42").unwrap();
    let json = r#"{"number":42,"title":"Title","body":"Body","url":"https://github.com/o/r/issues/42","labels":[{"name":"bug"}]}"#;
    let gh = FakeGh::new(vec![GhOutcome::Exit {
        code: 0,
        stdout: json.into(),
        stderr: String::new(),
    }]);
    let draft = import(&gh, &r, Duration::from_secs(1)).unwrap();
    assert_eq!(draft.labels, vec!["bug"]);
    assert!(draft_text(&draft, Lang::En).ends_with("Source: https://github.com/o/r/issues/42"));
    assert!(draft_text(&draft, Lang::ZhTw).ends_with("來源：https://github.com/o/r/issues/42"));
    assert_eq!(
        gh.calls(),
        vec![vec![
            "issue",
            "view",
            "42",
            "--repo",
            "o/r",
            "--json",
            "number,title,body,url,labels"
        ]]
    );
    for (outcome, expected) in [
        (GhOutcome::Missing, ImportError::GhMissing),
        (GhOutcome::TimedOut, ImportError::TimedOut),
        (
            GhOutcome::Exit {
                code: 1,
                stdout: "".into(),
                stderr: "run gh auth login".into(),
            },
            ImportError::NotLoggedIn,
        ),
        (
            GhOutcome::Exit {
                code: 1,
                stdout: "".into(),
                stderr: "Could not resolve issue".into(),
            },
            ImportError::NotFound,
        ),
        (
            GhOutcome::Exit {
                code: 1,
                stdout: "".into(),
                stderr: "Not found".into(),
            },
            ImportError::NotFound,
        ),
        (
            GhOutcome::Exit {
                code: 0,
                stdout: "{}".into(),
                stderr: "".into(),
            },
            ImportError::BadFormat,
        ),
        (
            GhOutcome::Exit {
                code: 0,
                stdout: "not json".into(),
                stderr: "".into(),
            },
            ImportError::BadFormat,
        ),
        (
            GhOutcome::Exit {
                code: 1,
                stdout: "".into(),
                stderr: "other".into(),
            },
            ImportError::Failed("other".into()),
        ),
    ] {
        assert_eq!(
            import(&FakeGh::new(vec![outcome]), &r, Duration::from_secs(1)),
            Err(expected)
        );
    }
}

#[test]
fn optimizer_arguments_are_pinned() {
    assert_eq!(
        claude_args(),
        &[
            "-p",
            "--model",
            "claude-sonnet-5-5",
            "--tools",
            "",
            "--strict-mcp-config",
            "--setting-sources",
            "local",
            "--output-format",
            "text"
        ]
    );
    assert_eq!(OPTIMIZE_TIMEOUT, Duration::from_secs(90));
}

#[test]
fn suggestions_cover_both_languages() {
    for (text, mode) in [
        ("How does this work?", ModeKind::Chat),
        ("這是什麼？", ModeKind::Chat),
        ("Make one small change", ModeKind::QuickTask),
        ("小改一下按鈕", ModeKind::QuickTask),
        ("Discuss the design", ModeKind::Plaza),
        ("請辯論並投票", ModeKind::Plaza),
        ("Build an application", ModeKind::Wish),
        ("建立完整的應用程式", ModeKind::Wish),
    ] {
        assert_eq!(suggest(text), mode, "{text}");
    }
}
