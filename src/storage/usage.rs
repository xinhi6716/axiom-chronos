use super::{Class, Entry, Ledger};
use crate::base::Res;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Debug, Default, Serialize)]
pub struct Bucket {
    pub bytes: u64,
    pub entries: usize,
    pub unmeasured: usize,
}
#[derive(Debug, Default, Serialize)]
pub struct Usage {
    pub by_class: BTreeMap<Class, Bucket>,
    pub by_project: BTreeMap<String, Bucket>,
    pub by_wish: BTreeMap<String, Bucket>,
    pub total: Bucket,
    pub reclaimable: Bucket,
    pub untracked: Vec<PathBuf>,
}
fn add(b: &mut Bucket, e: &Entry) {
    b.entries += 1;
    if let Some(n) = e.size_bytes {
        b.bytes = b.bytes.saturating_add(n);
    } else {
        b.unmeasured += 1;
    }
}
pub fn summarize(ledger: &Ledger, home: &Path, reclaimable: &dyn Fn(&Entry) -> bool) -> Res<Usage> {
    let mut u = Usage::default();
    for e in ledger.entries()? {
        add(u.by_class.entry(e.class).or_default(), &e);
        add(u.by_project.entry(e.owner.project.clone()).or_default(), &e);
        if let Some(w) = &e.owner.wish {
            add(
                u.by_wish
                    .entry(format!("{}/{}", e.owner.project, w))
                    .or_default(),
                &e,
            );
        }
        add(&mut u.total, &e);
        if reclaimable(&e) {
            add(&mut u.reclaimable, &e);
        }
    }
    u.untracked = ledger.unregistered(home)?;
    Ok(u)
}
pub fn render_text(u: &Usage) -> String {
    let mut out = String::new();
    fn line(out: &mut String, name: &str, b: &Bucket) {
        out.push_str(&format!(
            "{name}: {} bytes, {} entries, {} unmeasured\n",
            b.bytes, b.entries, b.unmeasured
        ));
    }
    out.push_str("Class:\n");
    for (k, v) in &u.by_class {
        line(&mut out, k.as_str(), v);
    }
    out.push_str("Project:\n");
    for (k, v) in &u.by_project {
        line(&mut out, k, v);
    }
    out.push_str("Wish:\n");
    for (k, v) in &u.by_wish {
        line(&mut out, k, v);
    }
    line(&mut out, "Total", &u.total);
    out.push_str("Reclaimable:\n");
    line(&mut out, "  Total", &u.reclaimable);
    out.push_str("Untracked:\n");
    for p in &u.untracked {
        out.push_str(&format!("{}\n", p.display()));
    }
    out
}
