pub mod approvals;
pub mod background;
pub mod dual_core;
pub mod extreme;
pub mod github;
pub mod interjection;
pub mod modes;
pub mod optimizer;
pub mod routines;
pub use routines::*;
pub mod paper; // p8:
pub mod research;
pub mod sessions;
pub mod storage;
pub mod unit_actions;
pub use approvals::*;
pub use background::*;
pub use dual_core::*;
pub use extreme::*;
pub use github::*;
pub use interjection::*;
pub use modes::*;
pub use optimizer::*;
pub use paper::*; // p8:
pub use sessions::*;
pub use storage::*;
pub use unit_actions::*;

#[cfg(test)]
mod research_tests {
    fn comments_only(source: &str) -> bool {
        source
            .lines()
            .map(str::trim)
            .all(|line| line.is_empty() || line.starts_with("//"))
    }

    #[test]
    fn research_is_a_comment_only_seam_without_a_reexport() {
        let seams = include_str!("mod.rs");
        assert!(seams.lines().any(|line| line.trim() == "pub mod research;"));
        assert!(!seams
            .lines()
            .any(|line| line.trim().starts_with("pub use research::")));
        assert!(comments_only(include_str!("research.rs")));
    }

    #[test]
    fn comments_only_rejects_items() {
        assert!(!comments_only("/// Documentation\npub fn example() {}\n"));
    }
}
