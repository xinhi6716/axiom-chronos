use crate::base::{Failure, Res};
pub use crate::research_program::dossier::Confidence;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

pub fn paper_dir(store: &Path) -> PathBuf {
    store.join("research").join("paper")
}
pub fn data_dir(data_home: &Path) -> PathBuf {
    data_home.join("paper")
}

macro_rules! enums {
    ($( $name:ident { $($variant:ident $(($ty:ty))? $( { $($field:ident : $fieldty:ty),* $(,)? } )?),* $(,)? } )*) => {$ (
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant $(($ty))? $( { $($field: $fieldty),* } )?),* }
    )*};
}
macro_rules! structs {
    ($( $name:ident { $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)? } )*) => {$ (
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
        pub struct $name { $($(#[$attr])* pub $field: $ty),* }
    )*};
}
enums! {
    SourceKind { Openalex, Sjr, Doaj, Crossref, JcrImport, OfficialPage, Program, Dataset, Person }
    Quartile { Q1, Q2, Q3, Q4 }
    QuartileScheme { Sjr, Jcr }
    Apc { NoFee, Fee(Money) }
    Access { OpenAccess, Hybrid, Subscription }
    AccessWanted { OpenAccess, Subscription, Either }
    ReqStatus { Met, Unmet, Unknown }
    Bucket { Passes, NeedsCheck }
    Chooser { Person }
    ArtifactKind { Dossier, Adjudication, Dataset, Model, Result }
    Aggregate { Mean, Sd, Se, Count, Sum, Min, Max, Median }
    Uncertainty { Sd(f64), Se(f64), Ci { level: f64, low: f64, high: f64 } }
    PlotKind { Line, Scatter, ErrorBar, Bar }
    ChangeKind { Added, Removed, Edited, Moved, Style }
    PolicyKind { Allowed, AllowedWithDisclosure, Prohibited }
    PaperEvent { Estimate(SpendEstimate), PassStarted { pass: PassId }, PassFinished { pass: PassId, status: crate::council::Status, changes: usize }, Check { step: Step, ok: bool, detail: String }, Blocked { step: Step, kind: String, reason: String }, Ready, Cancelled }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Known<T> {
    Known(Sourced<T>),
    Unknown { reason: String },
}
impl<T> Known<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Known(v) => Some(&v.value),
            Self::Unknown { .. } => None,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SourceTag,
}
structs! {
    SourceTag { kind: SourceKind, url: Option<String>, fetched_at: String, data_year: Option<i32>, raw: String }
    QuartileValue { quartile: Quartile, scheme: QuartileScheme, category: String, year: i32 }
    Money { amount_minor: u64, currency: String }
    JournalRecord { id: String, title: String, issns: Vec<String>, issn_l: Option<String>, publisher: Known<String>, homepage: Known<String>, quartiles: Vec<Sourced<QuartileValue>>, sjr: Known<f64>, access: Known<Access>, apc: Known<Apc>, review_weeks: Known<u32>, topics: Vec<String>, works_count: Known<u64> }
    Requirements { #[serde(rename = "abstract")] abstract_text: String, keywords: Vec<String>, max_quartile: Option<Quartile>, tier_scheme: QuartileScheme, access: AccessWanted, apc_limit: Option<Money>, require_no_fee: bool, categories: Vec<String>, free_text: Vec<String> }
    FilterResult { name: String, status: ReqStatus, detail: String, source: Option<SourceTag> }
    FitTerm { name: String, weight: f64, score: f64, evidence: String }
    FreeTextStatus { text: String, status: ReqStatus, detail: String, source: Option<SourceTag> }
    RankedJournal { journal: JournalRecord, bucket: Bucket, filters: Vec<FilterResult>, fit: f64, terms: Vec<FitTerm>, free_text: Vec<FreeTextStatus> }
    Excluded { journal: JournalRecord, filters: Vec<FilterResult> }
    Shortlist { requirements_hash: String, ranked: Vec<RankedJournal>, excluded: Vec<Excluded>, generated_at: String, hash: String }
    JournalChoice { journal_id: String, shortlist_hash: String, chosen_by: Chooser, at: String }
    ArtifactRef { path: String, sha256: String, kind: ArtifactKind, producer: String }
    Claim { id: String, text: String, evidence: Vec<String>, confidence: Confidence }
    EvidenceItem { id: String, source: String, artifact: Option<String>, locator: Option<String> }
    DataLocator { artifact: String, column: Option<String>, row: Option<u64>, aggregate: Option<Aggregate> }
    Quantity { id: String, claim_ids: Vec<String>, value: f64, unit: Option<String>, n: Option<u64>, uncertainty: Option<Uncertainty>, test: Option<String>, p_value: Option<f64>, precision: u8, locator: DataLocator }
    ResultsFile { producer: String, quantities: Vec<Quantity> }
    ReferenceSeed { key: String, doi: Option<String>, openalex: Option<String>, url: Option<String>, evidence: Vec<String> }
    EvidenceBundle { version: u32, program: String, question: String, artifacts: Vec<ArtifactRef>, claims: Vec<Claim>, evidence: Vec<EvidenceItem>, quantities: Vec<Quantity>, references: Vec<ReferenceSeed>, hash: String }
    ClaimLink { claim: String, evidence: Vec<String> }
    Paragraph { id: String, text: String, claims: Vec<ClaimLink> }
    Section { id: String, heading: String, paragraphs: Vec<Paragraph> }
    FigureSpec { id: String, kind: PlotKind, artifact: String, x: String, y: Vec<String>, error: String, preset: String, caption: Paragraph }
    TableColumn { column: String, header: String, precision: u8 }
    TableSpec { id: String, artifact: String, columns: Vec<TableColumn>, caption: Paragraph }
    Manuscript { title: String, #[serde(rename = "abstract")] abstract_text: Paragraph, keywords: Vec<String>, sections: Vec<Section>, figures: Vec<FigureSpec>, tables: Vec<TableSpec> }
    Invariants { claims: BTreeMap<String, Vec<ClaimLink>>, quantities: BTreeMap<String, Vec<String>>, citations: BTreeMap<String, Vec<String>> }
    Change { location: String, kind: ChangeKind, reason: String }
    PassOutput { manuscript: Manuscript, change_log: Vec<Change> }
    PassRecord { pass: PassId, member: String, model: Option<String>, provider: String, status: crate::council::Status, evidence_hash: String, input_hash: String, output_hash: Option<String>, usd: Option<f64>, tokens: u64, raw_path: String, change_log: Vec<Change>, validation: Vec<String>, at: String }
    StepRecord { step: Step, inputs: BTreeMap<String, String>, output: String, ok: bool, detail: Option<String>, at: String }
    PaperState { version: u32, run_id: String, steps: BTreeMap<Step, StepRecord> }
    RenderedTable { id: String, latex: String, cells: Vec<Vec<String>>, headers: Vec<String>, units: Vec<Option<String>>, width_mm: f64 }
    AiPolicy { publisher: String, url: String, fetched_at: String, sha256: String, policy: PolicyKind, placement: String, required_items: Vec<String> }
    ModelUse { pass: Option<PassId>, member: String, model: String, provider: String, purpose: String }
    Disclosure { text: String, placement: String, uses: Vec<ModelUse>, policy: AiPolicy }
    SpendEstimate { per_member: BTreeMap<String, Option<f64>>, priced_usd: Option<f64>, samples: usize, unpriced: Vec<String>, line: String, digest: String }
}
impl QuartileValue {
    pub fn label(&self) -> String {
        format!(
            "{} {} ({}, {})",
            match self.scheme {
                QuartileScheme::Sjr => "SJR",
                QuartileScheme::Jcr => "JCR",
            },
            match self.quartile {
                Quartile::Q1 => "Q1",
                Quartile::Q2 => "Q2",
                Quartile::Q3 => "Q3",
                Quartile::Q4 => "Q4",
            },
            self.category,
            self.year
        )
    }
}
fn json_hash(value: &Value) -> String {
    crate::seams::sha256_hex(&serde_json::to_vec(value).expect("JSON value serializes"))
}
impl EvidenceBundle {
    pub fn compute_hash(&self) -> String {
        let mut value = serde_json::to_value(self).expect("bundle serializes");
        value
            .as_object_mut()
            .expect("bundle is an object")
            .remove("hash");
        json_hash(&value)
    }
}
impl Manuscript {
    pub fn from_member_json(value: &Value) -> Res<Self> {
        crate::base::schema::validate(
            &serde_json::from_str(include_str!("../../engine/paper/pass.schema.json"))
                .expect("valid schema"),
            &serde_json::json!({"manuscript": value, "change_log": []}),
        )
        .map_err(|e| Failure::new("malformed", e.to_string()))?;
        serde_json::from_value(value.clone()).map_err(|e| Failure::new("malformed", e.to_string()))
    }
    pub fn to_member_json(&self) -> Value {
        serde_json::to_value(self).expect("manuscript serializes")
    }
    pub fn hash(&self) -> String {
        json_hash(&self.to_member_json())
    }
    pub fn paragraphs(&self) -> Vec<(String, &Paragraph)> {
        let mut result = vec![("abstract".into(), &self.abstract_text)];
        for section in &self.sections {
            for p in &section.paragraphs {
                result.push((format!("sec:{}/p:{}", section.id, p.id), p));
            }
        }
        for fig in &self.figures {
            result.push((format!("fig:{}/caption", fig.id), &fig.caption));
        }
        for tab in &self.tables {
            result.push((format!("tab:{}/caption", tab.id), &tab.caption));
        }
        result
    }
    pub fn invariants(&self) -> Invariants {
        let mut result = Invariants {
            claims: BTreeMap::new(),
            quantities: BTreeMap::new(),
            citations: BTreeMap::new(),
        };
        for (location, p) in self.paragraphs() {
            let mut claims = p.claims.clone();
            for claim in &mut claims {
                claim.evidence.sort();
            }
            claims.sort_by(|a, b| a.claim.cmp(&b.claim).then(a.evidence.cmp(&b.evidence)));
            let mut quantities = Vec::new();
            let mut citations = Vec::new();
            for (_, token) in tokens(&p.text) {
                match token {
                    Token::Quantity(q) => quantities.push(q),
                    Token::Cite(c) => citations.extend(c),
                    _ => {}
                }
            }
            quantities.sort();
            citations.sort();
            result.claims.insert(location.clone(), claims);
            result.quantities.insert(location.clone(), quantities);
            result.citations.insert(location, citations);
        }
        result
    }
}
enums! { Token { Quantity(String), Cite(Vec<String>), Fig(String), Tab(String), Sec(String) } }
pub fn tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    use std::sync::OnceLock;
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let re = PATTERN.get_or_init(|| regex::Regex::new(r"\{\{(q|fig|tab|sec):([A-Za-z0-9_.:-]+)\}\}|\[@[A-Za-z0-9_.:-]+(?:;\s*@[A-Za-z0-9_.:-]+)*\]").unwrap());
    re.captures_iter(text)
        .map(|c| {
            let m = c.get(0).unwrap();
            let token = if let Some(kind) = c.get(1) {
                let id = c[2].to_owned();
                match kind.as_str() {
                    "q" => Token::Quantity(id),
                    "fig" => Token::Fig(id),
                    "tab" => Token::Tab(id),
                    _ => Token::Sec(id),
                }
            } else {
                Token::Cite(
                    m.as_str()[1..m.as_str().len() - 1]
                        .split(';')
                        .map(|s| s.trim().strip_prefix('@').unwrap().to_owned())
                        .collect(),
                )
            };
            (m.start()..m.end(), token)
        })
        .collect()
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PassId {
    AstraDraft,
    FableRevise,
    ArgonRevise,
    FableStyle,
}
impl PassId {
    pub const ALL: [Self; 4] = [
        Self::AstraDraft,
        Self::FableRevise,
        Self::ArgonRevise,
        Self::FableStyle,
    ];
    pub fn number(&self) -> u8 {
        match self {
            Self::AstraDraft => 1,
            Self::FableRevise => 2,
            Self::ArgonRevise => 3,
            Self::FableStyle => 4,
        }
    }
    pub fn member(&self) -> &'static str {
        match self {
            Self::AstraDraft => "astra",
            Self::FableRevise | Self::FableStyle => "fable",
            Self::ArgonRevise => "argon",
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Evidence,
    Shortlist,
    Journal,
    Pass1,
    Pass2,
    Pass3,
    Pass4,
    Figures,
    Citations,
    Numbers,
    Compile,
    Render,
    Disclosure,
    Review,
    Ready,
}
