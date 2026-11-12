use axiom::{
    base::Res,
    graph::{
        code::{sync, SyncOptions},
        dream::{command, dream, DreamOptions, SemanticCheck},
        GraphStore,
    },
    usage::CodexTokens,
};
use rusqlite::{params, Connection};
use std::{fs, time::Duration};

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        GraphStore::open(home.path()).unwrap();
        Self { home, project }
    }
    fn db(&self) -> Connection {
        Connection::open(self.home.path().join("memory/memory.db")).unwrap()
    }
    fn add(&self, title: &str, summary: &str, body: &str, files: &[&str], symbols: &str) -> i64 {
        let db = self.db();
        db.execute("INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created,confirmed) VALUES('demo','u',?,?,?,?, '',?,?,1,1)",params![title,summary,body,symbols,serde_json::to_string(files).unwrap(),self.project.path().to_string_lossy().as_ref()]).unwrap();
        db.last_insert_rowid()
    }
    fn prep(&self) {
        GraphStore::open(self.home.path())
            .unwrap()
            .process_dirty()
            .unwrap();
    }
    fn run(&self) -> axiom::graph::dream::DreamReport {
        dream(self.home.path(), &self.opts()).unwrap()
    }
    fn opts(&self) -> DreamOptions<'static> {
        let mut o = DreamOptions::new("demo");
        o.project_dir = Some(self.project.path().into());
        o
    }
    fn file(&self, path: &str, content: &str) {
        let p = self.project.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
    fn index(&self) {
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(self.project.path())
            .status()
            .unwrap();
        assert!(status.success());
        sync(
            &GraphStore::open(self.home.path()).unwrap(),
            self.project.path(),
            None,
            &SyncOptions::default(),
        )
        .unwrap();
    }
    fn row(&self, id: i64) -> Vec<String> {
        self.db().query_row("SELECT project,unit,title,summary,body,symbols,wish,files,project_dir,CAST(created AS TEXT),COALESCE(CAST(confirmed AS TEXT),'null'),COALESCE(CAST(corrected AS TEXT),'null') FROM entries WHERE id=?",[id],|r|(0..12).map(|i|r.get(i)).collect()).unwrap()
    }
    fn state(&self, id: i64) -> String {
        self.db()
            .query_row("SELECT state FROM g_nodes WHERE entry_id=?", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }
}
fn ops(r: &axiom::graph::dream::DreamReport) -> Vec<&str> {
    r.changes.iter().map(|c| c.op.as_str()).collect()
}
#[test]
fn duplicate_and_repeat() {
    let f = Fixture::new();
    let a = f.add("same", "sum", "body", &[], "");
    let b = f.add("same", "sum", "body", &[], "");
    f.prep();
    let before = (f.row(a), f.row(b));
    let r = f.run();
    assert!(ops(&r).contains(&"merge_duplicate"));
    assert!(ops(&r).contains(&"index_node"));
    assert_eq!(f.state(b), "merged");
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM g_changes WHERE actor='dream' AND op='merge_duplicate'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(before, (f.row(a), f.row(b)));
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM g_edges WHERE kind='merged_into'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(f.run().changes.is_empty());
}
#[test]
fn stale_and_negative() {
    let f = Fixture::new();
    f.file("ok.rs", "pub fn okay() {}\n");
    let good = f.add("good", "sum", "refer ok.rs and okay", &["ok.rs"], "okay");
    let stale = f.add("old", "sum", "old stuff", &["gone.rs"], "lost");
    f.index();
    f.prep();
    let row = f.row(good);
    let r = f.run();
    assert!(r
        .changes
        .iter()
        .any(|c| c.op == "retire_stale" && c.target == format!("entry:{stale}")));
    assert!(!r
        .changes
        .iter()
        .any(|c| c.target == format!("entry:{good}")));
    assert_eq!(row, f.row(good));
    assert_eq!(f.state(stale), "retired");
}
#[test]
fn fix_single_token() {
    let f = Fixture::new();
    f.file("new/foo.rs", "pub fn value() {}\n");
    let id = f.add(
        "fix",
        "old",
        "\n  before old/foo.rs after  \nsecond line",
        &["old/foo.rs"],
        "",
    );
    f.index();
    f.prep();
    let before = f.row(id);
    let r = f.run();
    assert!(ops(&r).contains(&"fix_error"));
    let after = f.row(id);
    assert_eq!(after[4], "\n  before new/foo.rs after  \nsecond line");
    assert_eq!(after[3], "before new/foo.rs after");
    assert_ne!(after[10], "null");
    assert_ne!(after[10], before[10]);
    assert_ne!(after[11], "null");
    assert_eq!(
        f.db().query_row(
            "SELECT count(*) FROM g_changes WHERE actor='dream' AND op='fix_error' AND target=?",
            [format!("entry:{id}")],
            |r| r.get::<_, i64>(0),
        ).unwrap(),
        1
    );
    for i in [0, 1, 2, 5, 6, 7, 8, 9] {
        assert_eq!(before[i], after[i]);
    }
    assert!(f.run().changes.is_empty());
}
#[test]
fn rejected_fix_marks_entry_unconfirmed() {
    let f = Fixture::new();
    f.file("new/foo.rs", "pub fn value() {}\n");
    let body = format!("old/foo.rs\n{}", "extra\n".repeat(12));
    let id = f.add("guarded", "sum", &body, &["old/foo.rs"], "");
    f.index();
    f.prep();
    let before = f.row(id);
    let report = f.run();
    assert!(report
        .changes
        .iter()
        .any(|c| c.op == "mark_unconfirmed" && c.target == format!("entry:{id}")));
    assert!(!ops(&report).contains(&"fix_error"));
    assert!(!ops(&report).contains(&"retire_stale"));
    assert_eq!(f.state(id), "unconfirmed");
    assert_eq!(f.row(id), before);
    assert!(f.run().changes.is_empty());
}
#[test]
fn retired_entry_with_link_does_not_change_on_second_run() {
    let f = Fixture::new();
    let id = f.add("old", "sum", "old stuff", &["gone.rs"], "lost");
    f.prep();
    let db = f.db();
    let (entry, scope): (i64, i64) = db
        .query_row(
            "SELECT id, scope FROM g_nodes WHERE entry_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let target = node(&db, scope, "unit:linked", "unit", "linked", "active");
    db.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'test','','','1','active',0)", [scope]).unwrap();
    let source = db.last_insert_rowid();
    db.execute(
        "INSERT INTO g_edges VALUES(?,?,'cites',?,'',1000,'active','{}')",
        params![entry, target, source],
    )
    .unwrap();
    drop(db);
    let first = f.run();
    assert!(first
        .changes
        .iter()
        .any(|c| c.op == "retire_stale" && c.target == format!("entry:{id}")));
    assert_eq!(f.state(id), "retired");
    let second = f.run();
    assert!(second.changes.is_empty(), "{:?}", second.changes);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT state FROM g_edges WHERE src=? AND dst=?",
                params![entry, target],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "active"
    );
}
#[test]
fn ambiguous_path() {
    let f = Fixture::new();
    f.file("a/foo.rs", "pub fn one() {}\n");
    f.file("b/foo.rs", "pub fn two() {}\n");
    let id = f.add("amb", "sum", "old/foo.rs", &["old/foo.rs"], "");
    f.index();
    f.prep();
    let row = f.row(id);
    let r = f.run();
    assert!(ops(&r).contains(&"mark_unconfirmed"));
    assert!(!ops(&r).contains(&"fix_error"));
    assert_eq!(f.state(id), "unconfirmed");
    assert_eq!(row, f.row(id));
}
fn node(db: &Connection, scope: i64, key: &str, kind: &str, label: &str, state: &str) -> i64 {
    db.execute("INSERT INTO g_nodes(scope,key,kind,label,summary,attrs,state,revision) VALUES(?,?,?,?,'','{}',?,0)",params![scope,key,kind,label,state]).unwrap();
    db.last_insert_rowid()
}
fn edge_case(multiple: bool) {
    let f = Fixture::new();
    f.file("new.rs", "");
    f.file("new2.rs", "");
    let store = GraphStore::open(f.home.path()).unwrap();
    let scope = store.scope(f.project.path(), None).unwrap().id;
    let db = f.db();
    let a = node(&db, scope, "finding:x", "finding", "finding", "active");
    let old = node(&db, scope, "file:old.rs", "file", "target", "retired");
    let replacement = node(&db, scope, "file:new.rs", "file", "target", "active");
    if multiple {
        node(&db, scope, "file:new2.rs", "file", "target", "active");
    }
    db.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'test','','','1','active',0)",[scope]).unwrap();
    let source = db.last_insert_rowid();
    db.execute(
        "INSERT INTO g_edges VALUES(?,?,'cites',?,'',1000,'active','{}')",
        params![a, old, source],
    )
    .unwrap();
    drop(db);
    let r = f.run();
    if multiple {
        assert!(ops(&r).contains(&"mark_unconfirmed"));
        assert_eq!(
            f.db()
                .query_row("SELECT state FROM g_edges WHERE dst=?", [old], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "unconfirmed"
        );
    } else {
        assert!(ops(&r).contains(&"repair_link"));
        assert_eq!(
            f.db()
                .query_row("SELECT dst FROM g_edges WHERE src=?", [a], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            replacement
        );
    }
    assert!(f.run().changes.is_empty());
}
#[test]
fn links_unique_and_ambiguous() {
    edge_case(false);
    edge_case(true);
}
fn snapshot(db: &Connection) -> Vec<String> {
    let mut out = vec![];
    for table in [
        "entries",
        "g_meta",
        "g_scopes",
        "g_sources",
        "g_nodes",
        "g_edges",
        "g_files",
        "g_refs",
        "g_aliases",
        "g_dirty_entries",
        "g_changes",
    ] {
        let mut stmt = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let count = stmt.column_count();
        let rows = stmt
            .query_map([], |r| {
                let mut v = vec![];
                for i in 0..count {
                    v.push(format!("{:?}", r.get_ref(i)?));
                }
                Ok(format!("{table}:{v:?}"))
            })
            .unwrap();
        out.extend(rows.map(Result::unwrap));
    }
    out.sort();
    out
}
#[test]
fn dry_run_and_limits() {
    let f = Fixture::new();
    f.add("dup", "sum", "body", &[], "");
    f.add("dup", "sum", "body", &[], "");
    f.prep();
    let before = snapshot(&f.db());
    let mut o = f.opts();
    o.dry_run = true;
    let r = dream(f.home.path(), &o).unwrap();
    assert!(ops(&r).contains(&"merge_duplicate"));
    assert_eq!(before, snapshot(&f.db()));
    o.dry_run = false;
    o.max_changes = 1;
    let r = dream(f.home.path(), &o).unwrap();
    assert_eq!(r.changes.len(), 1);
    assert_eq!(r.stopped.as_deref(), Some("max_changes"));
    let before = snapshot(&f.db());
    o.deadline = Duration::ZERO;
    let r = dream(f.home.path(), &o).unwrap();
    assert_eq!(r.stopped.as_deref(), Some("deadline"));
    assert_eq!(before, snapshot(&f.db()));
}
struct Fake(bool);
impl SemanticCheck for Fake {
    fn check(&self, _: &str) -> Res<(String, Option<CodexTokens>)> {
        Ok((
            "ok".into(),
            self.0.then_some(CodexTokens {
                input: 2,
                output: 3,
                total: 5,
                ..Default::default()
            }),
        ))
    }
}
#[test]
fn usage_and_command() {
    let f = Fixture::new();
    f.add("a", "x", "a", &[], "");
    f.add("b", "x", "b", &[], "");
    f.prep();
    let yes = Fake(true);
    let no = Fake(false);
    let mut o = f.opts();
    o.dry_run = true;
    o.semantic = Some(&yes);
    let r = dream(f.home.path(), &o).unwrap();
    assert_eq!(r.codex_usage.unwrap().total, 10);
    o.semantic = Some(&no);
    assert!(dream(f.home.path(), &o).unwrap().codex_usage.is_none());
    o.semantic = None;
    assert!(dream(f.home.path(), &o).unwrap().codex_usage.is_none());
    assert_eq!(command(&[]), 2);
}
#[test]
fn valid_command_is_isolated() {
    let f = Fixture::new();
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .env("AXIOM_DREAM_COMMAND_CHILD", "1")
        .env("AXIOM_HOME", f.home.path())
        .arg("--exact")
        .arg("command_child")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn command_child() {
    if std::env::var_os("AXIOM_DREAM_COMMAND_CHILD").is_some() {
        assert_eq!(
            command(&["--project".into(), "demo".into(), "--json".into()]),
            0
        )
    }
}

#[test]
fn zero_deadline_does_not_process_dirty() {
    let f = Fixture::new();
    f.add("pending", "sum", "body", &[], "");
    let before = snapshot(&f.db());
    let mut o = f.opts();
    o.deadline = Duration::ZERO;
    let r = dream(f.home.path(), &o).unwrap();
    assert_eq!(r.stopped.as_deref(), Some("deadline"));
    assert_eq!(before, snapshot(&f.db()));
}

#[test]
fn dry_run_reports_pending_duplicates_without_processing_dirty() {
    let f = Fixture::new();
    f.add("same", "sum", "body", &[], "");
    f.add("same", "sum", "body", &[], "");
    let before = snapshot(&f.db());
    let mut o = f.opts();
    o.dry_run = true;
    let r = dream(f.home.path(), &o).unwrap();
    assert!(ops(&r).contains(&"merge_duplicate"));
    assert_eq!(before, snapshot(&f.db()));
}
#[test]
fn fixes_one_symbol_reference() {
    let f = Fixture::new();
    f.file("source.rs", "pub fn value() {}\n");
    let id = f.add(
        "symbol",
        "old",
        "call old::value here",
        &["source.rs"],
        "old::value",
    );
    f.index();
    f.prep();
    let report = f.run();
    assert!(report
        .changes
        .iter()
        .any(|c| c.op == "fix_error" && c.target == format!("entry:{id}")));
    assert_eq!(f.row(id)[4], "call value here");
    assert!(f.run().changes.is_empty());
}

#[test]
fn dry_run_new_home_does_not_create_database() {
    let home = tempfile::tempdir().unwrap();
    let o = DreamOptions {
        dry_run: true,
        ..DreamOptions::new("demo")
    };
    let report = dream(home.path(), &o).unwrap();
    assert_eq!(ops(&report), vec!["index_node"]);
    assert!(!home.path().join("memory").exists());
}

fn link_entry_to_file(f: &Fixture, id: i64) {
    f.file("linked.rs", "pub fn linked() {}\n");
    let db = f.db();
    let (entry, scope): (i64, i64) = db.query_row(
        "SELECT id, scope FROM g_nodes WHERE entry_id=?", [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    let target = node(&db, scope, "file:linked.rs", "file", "linked.rs", "active");
    db.execute("INSERT INTO g_sources(scope,kind,locator,sha256,version,state,observed_at) VALUES(?,'test','','','1','active',0)", [scope]).unwrap();
    let source = db.last_insert_rowid();
    db.execute("INSERT INTO g_edges VALUES(?,?,'cites',?,'',1000,'active','{}')", params![entry, target, source]).unwrap();
}

#[test]
fn linked_ambiguous_entry_second_run_has_no_changes() {
    let f = Fixture::new();
    f.file("a/foo.rs", "pub fn one() {}\n");
    f.file("b/foo.rs", "pub fn two() {}\n");
    let id = f.add("amb", "sum", "old/foo.rs", &["old/foo.rs"], "");
    f.index();
    f.prep();
    link_entry_to_file(&f, id);
    let first = f.run();
    assert!(first.changes.iter().any(|c| c.op == "mark_unconfirmed" && c.target == format!("entry:{id}") && c.reason == "ambiguous replacement"));
    assert_eq!(f.state(id), "unconfirmed");
    let second = f.run();
    assert!(second.changes.is_empty(), "{:?}", second.changes);
}

#[test]
fn linked_rejected_fix_entry_second_run_has_no_changes() {
    let f = Fixture::new();
    f.file("new/foo.rs", "pub fn value() {}\n");
    let body = format!("old/foo.rs\n{}", "extra\n".repeat(12));
    let id = f.add("guarded", "sum", &body, &["old/foo.rs"], "");
    f.index();
    f.prep();
    link_entry_to_file(&f, id);
    let first = f.run();
    assert!(first.changes.iter().any(|c| c.op == "mark_unconfirmed" && c.target == format!("entry:{id}") && c.reason == "replacement rejected by correction checks"));
    assert_eq!(f.state(id), "unconfirmed");
    let second = f.run();
    assert!(second.changes.is_empty(), "{:?}", second.changes);
}

#[test]
fn linked_merged_entry_second_run_has_no_changes() {
    let f = Fixture::new();
    f.add("same", "sum", "body", &[], "");
    let id = f.add("same", "sum", "body", &[], "");
    f.prep();
    link_entry_to_file(&f, id);
    let first = f.run();
    assert!(first.changes.iter().any(|c| c.op == "merge_duplicate" && c.target == format!("entry:{id}")));
    assert_eq!(f.state(id), "merged");
    let second = f.run();
    assert!(second.changes.is_empty(), "{:?}", second.changes);
}

#[test]
fn linked_retired_entry_second_run_has_no_changes() {
    let f = Fixture::new();
    let id = f.add("old", "sum", "old stuff", &["gone.rs"], "lost");
    f.prep();
    link_entry_to_file(&f, id);
    let first = f.run();
    assert!(first.changes.iter().any(|c| c.op == "retire_stale" && c.target == format!("entry:{id}")));
    assert_eq!(f.state(id), "retired");
    let second = f.run();
    assert!(second.changes.is_empty(), "{:?}", second.changes);
}
