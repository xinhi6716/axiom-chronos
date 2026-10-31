use axiom::memory::Memory;
use rusqlite::Connection;
use std::time::Instant;

#[test]
#[ignore = "deterministic local retrieval probe"]
fn retrieval_probe() {
    let temp = tempfile::tempdir().unwrap();
    let memory = Memory::open(temp.path()).unwrap();
    let mut db = Connection::open(temp.path().join("memory/memory.db")).unwrap();
    let tx = db.transaction().unwrap();
    for i in 0..1000 {
        let title = if i % 10 == 0 {
            "統計 target"
        } else {
            "other"
        };
        tx.execute("INSERT INTO entries VALUES (?1,'p','u',?2,'summary','body','','wish','[]','missing',?1)", (i + 1, title)).unwrap();
    }
    tx.commit().unwrap();
    let start = Instant::now();
    let result = memory.search("統計 target", Some("p"), 10).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(result["entries"].as_array().unwrap().len(), 10);
    assert_eq!(result["entries"][0]["id"], 991);
    println!(
        "rows=1000 matches=100 returned=10 query_bytes={} elapsed_us={}",
        "統計 target".len(),
        elapsed.as_micros()
    );
}
