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
#[allow(unused_imports)]
pub use research::*;
pub use sessions::*;
pub use storage::*;
pub use unit_actions::*;

#[cfg(test)]
mod research_tests {
    fn base_only(source: &str) -> bool {
        source
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("use ") || line.starts_with("pub use "))
            .all(|line| {
                line.starts_with("use crate::base") || line.starts_with("pub use crate::base")
            })
    }

    #[test]
    fn research_seam_is_declared_and_base_only() {
        let seams = include_str!("mod.rs");
        assert!(seams.lines().any(|line| line.trim() == "pub mod research;"));
        assert!(base_only(include_str!("research.rs")));
    }

    #[test]
    fn base_only_rejects_other_imports() {
        assert!(!base_only("use crate::desktop::ux::Foo;\n"));
        assert!(base_only("use crate::base::Res;\n"));
    }
}
