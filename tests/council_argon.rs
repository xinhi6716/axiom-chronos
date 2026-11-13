mod common;
mod council {
    use super::common;
    use axiom::council::{self, CrossReply, CrossRequest, Setup, Status};
    use regex::Regex;
    use serde_json::{json, Value};
    use std::{
        fs,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Mutex,
        },
    };
    const BRIEF: &str = "GOAL the changes that carry out the wish\nWISH make it faster";
    use axiom::council::{
        budgets::Budgets,
        settings::{self, CouncilSettings},
    };

    #[test]
    fn settings_round_trip_and_override_parsing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(settings::load(dir.path()).unwrap().argon_every_round);
        settings::save(
            dir.path(),
            &CouncilSettings {
                argon_every_round: false,
            },
        )
        .unwrap();
        assert!(!settings::load(dir.path()).unwrap().argon_every_round);
        assert_eq!(
            settings::parse_argon_every_round(Some("1".into())).unwrap(),
            Some(true)
        );
        assert_eq!(
            settings::parse_argon_every_round(Some("0".into())).unwrap(),
            Some(false)
        );
        assert!(settings::parse_argon_every_round(Some("yes".into())).is_err());
        assert!(Budgets::default()
            .notice_for(true)
            .contains("per call across rounds"));
    }

    struct Case {
        temp: tempfile::TempDir,
        plan: PathBuf,
        schema: PathBuf,
        log: PathBuf,
    }
    impl Case {
        fn new() -> Self {
            let temp = common::temp();
            let dir = temp.path().join("orchestrate/project/astra");
            fs::create_dir_all(&dir).unwrap();
            fs::create_dir_all(temp.path().join("orchestrate/project/reports")).unwrap();
            fs::create_dir_all(temp.path().join("storage")).unwrap();
            fs::write(
                temp.path().join("storage/policy.json"),
                r#"{"reserve_bytes":0}"#,
            )
            .unwrap();
            let plan = dir.join("plan.json");
            fs::write(
            &plan,
            serde_json::to_vec(&json!({
                "summary":"astra", "coverage":{"read":[],"not_read":[]}, "out_of_scope":[],
                "changes":[
                    {"id":"a1","area":"design","location":"src/a.py","evidence":"astra evidence one","problem":"slow","change":"cache it","verify":"time it","tier":"sol","effort":"high","risk":"standard","reason":"Bounded cache change"},
                    {"id":"a2","area":"tests","location":"src/b.py","evidence":"astra evidence two","problem":"flaky","change":"pin it","verify":"run it","tier":"sol","effort":"high","risk":"standard","reason":"Bounded test change"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
            let schema = common::repo_root().join("engine/research/audit.schema.json");
            let log = temp.path().join("calls.jsonl");
            Self {
                temp,
                plan,
                schema,
                log,
            }
        }
        fn member(&self, scenario: &str) -> axiom::base::Res<Vec<String>> {
            let mut argv = common::fake("fake_member");
            argv.push(format!("--fake-scenario={scenario}"));
            argv.push(format!("--fake-log={}", self.log.display()));
            Ok(argv)
        }
        fn setup(&self, fable: &str, argon: &str) -> Setup {
            let mut setup = Setup::new(self.member(fable), self.member(argon));
            setup.argon_every_round = false;
            setup
        }
        fn request(&self) -> council::Request {
            council::Request {
                brief: BRIEF.into(),
                cwd: self.temp.path().to_path_buf(),
                plan_path: self.plan.clone(),
                schema_path: self.schema.clone(),
            }
        }
        fn calls(&self) -> Vec<Value> {
            fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
        fn run(&self, setup: &Setup, astra: &Astra) -> axiom::base::Res<Value> {
            let cross = |r: &CrossRequest| astra.answer(r);
            council::run(setup, &self.request(), &cross, &AtomicBool::new(false))
        }
    }

    struct Astra {
        status: Status,
        calls: AtomicUsize,
        prompt: Mutex<String>,
    }
    impl Astra {
        fn with(status: Status) -> Self {
            Self {
                status,
                calls: AtomicUsize::new(0),
                prompt: Mutex::new(String::new()),
            }
        }
        fn ok() -> Self {
            Self::with(Status::Ok)
        }
        fn answer(&self, request: &CrossRequest) -> CrossReply {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.prompt.lock().unwrap() = request.prompt.clone();
            let ids = Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
            let stances: Vec<Value> = ids
                .captures_iter(&request.prompt)
                .map(|c| json!({"claim":c[1].to_string(),"stance":"agree","evidence":""}))
                .collect();
            CrossReply {
                status: self.status,
                raw: None,
                output: (self.status == Status::Ok).then(|| json!({ "stances": stances })),
                tokens: 321,
                model: Some("gpt-6-astra".into()),
                detail: (self.status != Status::Ok).then(|| "not this time".to_string()),
            }
        }
    }

    fn every_round(on: bool) {
        let case = Case::new();
        let mut setup = case.setup("disagree,resolve-a", "disagree");
        setup.argon_every_round = on;
        let reply = case.run(&setup, &Astra::ok()).unwrap();
        let rounds = reply["rounds"].as_array().unwrap();
        for phase in ["A", "B", "C"] {
            assert!(
                rounds.iter().any(|r| r["phase"] == phase),
                "phase {phase} ran"
            );
        }
        assert!(rounds.iter().filter(|r| r["phase"] == "A").count() >= 2);
        let calls = case.calls();
        let stored: Value =
            serde_json::from_slice(&fs::read(case.plan.with_file_name("council.json")).unwrap())
                .unwrap();
        for round in rounds {
            let phase = round["phase"].as_str().unwrap();
            let n = round["n"].as_u64().unwrap();
            let saved = stored["rounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["phase"] == phase && r["n"] == n)
                .unwrap();
            assert!(
                saved["turns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["member"] == "argon"),
                "argon recorded turn in {phase} {n}"
            );
            assert!(
                round["speakers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s == "argon"),
                "argon turn in {phase} {n}"
            );
            assert!(
                calls
                    .iter()
                    .any(|c| c["role"] == "argon" && c["phase"] == phase && c["round"] == n),
                "argon call in {phase} {n}"
            );
        }
    }
    #[test]
    fn argon_speaks_in_every_a_b_c_round() {
        every_round(true);
    }
    #[test]
    #[should_panic]
    fn every_round_negative_control() {
        every_round(false);
    }

    fn usage(on: bool) {
        use axiom::usage::pools::{self, Pool};
        let case = Case::new();
        let mut setup = case.setup("ok", "ok");
        setup.argon_every_round = on;
        case.run(&setup, &Astra::ok()).unwrap();
        let statuses = pools::status(case.temp.path(), 0).unwrap();
        let used = |pool| {
            statuses
                .iter()
                .find(|s| s.pool == pool)
                .unwrap()
                .used_tokens
                .unwrap()
        };
        assert_eq!(used(Pool::HydrogenFlash), 0);
        // Off still charges its single B review; distinguish the additional A calls.
        assert!(
            used(Pool::Argon) > 1234,
            "argon usage includes every-round calls"
        );
    }
    #[test]
    fn argon_usage_is_charged_to_the_argon_pool_only() {
        usage(true);
    }
    #[test]
    #[should_panic]
    fn usage_negative_control() {
        usage(false);
    }

    #[test]
    fn off_keeps_old_protocol() {
        let case = Case::new();
        let result = case
            .run(&case.setup("disagree,resolve-a", "disagree"), &Astra::ok())
            .unwrap();
        assert_eq!(
            case.calls().iter().filter(|c| c["role"] == "argon").count(),
            1
        );
        for r in result["rounds"].as_array().unwrap() {
            assert_eq!(
                r["speakers"],
                if r["phase"] == "B" {
                    json!(["argon"])
                } else {
                    json!(["astra", "fable"])
                }
            );
        }
    }
    #[test]
    fn setting_changes_cache_key() {
        let case = Case::new();
        let mut setup = case.setup("ok", "ok");
        case.run(&setup, &Astra::ok()).unwrap();
        assert_eq!(case.run(&setup, &Astra::ok()).unwrap()["reused"], true);
        setup.argon_every_round = true;
        assert_eq!(case.run(&setup, &Astra::ok()).unwrap()["reused"], false);
    }
    fn malformed(on: bool, phase: &str) {
        for twice in [false, true] {
            let case = Case::new();
            let source =
                fs::read_to_string(common::repo_root().join("tests/fakes/fake_member.py")).unwrap();
            let source = source.replace("\r\n", "\n");
            let injected = format!("if {} and not is_fable and phase == '{phase}' and round_n == 1 and ({} or 'REPAIR: The previous review failed validation:' not in prompt):\n    print('not JSON')\n    sys.exit(0)\n\n", if on { "True" } else { "False" }, if twice { "True" } else { "False" });
            let script = case.temp.path().join("member.py");
            fs::write(
                &script,
                source.replace(
                    "if not is_fable:\n    repair =",
                    &(injected + "if not is_fable:\n    repair ="),
                ),
            )
            .unwrap();
            let mut setup = case.setup("disagree,resolve-a", "disagree");
            setup.argon_every_round = true;
            setup.argon = Ok(vec![
                common::python(),
                "-I".into(),
                script.to_string_lossy().into_owned(),
                "--fake-scenario=disagree".into(),
                format!("--fake-log={}", case.log.display()),
            ]);
            let events = Mutex::new(Vec::<Value>::new());
            let astra = Astra::ok();
            let result = council::run_with_events(
                &setup,
                &case.request(),
                &|r| astra.answer(r),
                &AtomicBool::new(false),
                &|event| events.lock().unwrap().push(event.clone()),
            )
            .unwrap();
            let calls = case.calls();
            assert!(
                calls
                    .iter()
                    .any(|c| c["role"] == "argon" && c["phase"] == "A"),
                "argon participates in A"
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|c| c["role"] == "argon" && c["phase"] == phase && c["round"] == 1)
                    .count(),
                2,
                "one repair in {phase}"
            );
            if twice {
                assert!(events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e["subtype"] == "member_degraded"
                        && e["member"] == "argon"
                        && e["phase"] == phase));
                let work = case.plan.parent().unwrap().join("council");
                assert!(work.join(format!("{phase}-r1-argon.raw.txt")).exists());
                assert!(work
                    .join(format!("{phase}-r1-argon.repair.raw.txt"))
                    .exists());
                assert!(result["degraded_members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["member"] == "argon" && d["phase"] == phase));
            } else {
                assert!(!result["degraded_members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["member"] == "argon" && d["phase"] == phase));
            }
        }
    }
    #[test]
    fn malformed_a() {
        malformed(true, "A");
    }
    #[test]
    #[should_panic(expected = "one repair in")]
    fn malformed_a_negative() {
        malformed(false, "A");
    }
    #[test]
    fn malformed_b() {
        malformed(true, "B");
    }
    #[test]
    #[should_panic(expected = "one repair in")]
    fn malformed_b_negative() {
        malformed(false, "B");
    }
    #[test]
    fn malformed_c() {
        malformed(true, "C");
    }
    #[test]
    #[should_panic(expected = "one repair in")]
    fn malformed_c_negative() {
        malformed(false, "C");
    }
}
