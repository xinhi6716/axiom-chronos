use super::Breakpoint;
use crate::desktop::theme::{space, Density, Step};
use gpui_kit::Pixels;

pub fn columns(bp: Breakpoint) -> u8 {
    match bp {
        Breakpoint::Narrow => 4,
        Breakpoint::Compact => 8,
        Breakpoint::Regular | Breakpoint::Wide => 12,
        Breakpoint::Ultra => 16,
    }
}
pub fn gutter(bp: Breakpoint, d: Density) -> Pixels {
    space(
        d,
        if bp < Breakpoint::Regular {
            Step::Md
        } else {
            Step::Lg
        },
    )
}
pub fn margin(bp: Breakpoint, d: Density) -> Pixels {
    space(
        d,
        if bp < Breakpoint::Regular {
            Step::Lg
        } else {
            Step::Xl
        },
    )
}
