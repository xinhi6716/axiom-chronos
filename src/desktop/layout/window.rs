use gpui_kit::{point, px, Bounds, Pixels, Size};

pub fn recover(
    saved: Option<Bounds<Pixels>>,
    displays: &[Bounds<Pixels>],
    default: Size<Pixels>,
    min: Size<Pixels>,
) -> Bounds<Pixels> {
    let valid_rect = |b: &Bounds<Pixels>| {
        let (x, y, w, h) = (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        );
        x.is_finite()
            && y.is_finite()
            && w.is_finite()
            && h.is_finite()
            && w > 0.
            && h > 0.
            && (x + w).is_finite()
            && (y + h).is_finite()
    };
    if let Some(b) = saved {
        if valid_rect(&b) && b.size.width >= min.width && b.size.height >= min.height && {
            // Use f64 for area arithmetic, including very large but finite bounds.
            let (x, y, w, h) = (
                f32::from(b.origin.x) as f64,
                f32::from(b.origin.y) as f64,
                f32::from(b.size.width) as f64,
                f32::from(b.size.height) as f64,
            );
            let overlap = |d: &Bounds<Pixels>, width: f64, height: f64| {
                let dx = f32::from(d.origin.x) as f64;
                let dy = f32::from(d.origin.y) as f64;
                let dw = f32::from(d.size.width) as f64;
                let dh = f32::from(d.size.height) as f64;
                ((x + width).min(dx + dw) - x.max(dx)).max(0.)
                    * ((y + height).min(dy + dh) - y.max(dy)).max(0.)
            };
            let visible = displays
                .iter()
                .filter(|d| valid_rect(d))
                .any(|d| overlap(d, w, h) >= w * h / 2.);
            let title_visible = displays
                .iter()
                .filter(|d| valid_rect(d))
                .any(|d| overlap(d, w.min(48.), h.min(32.)) > 0.);
            visible && title_visible
        } {
            return b;
        }
    }
    let origin = displays
        .first()
        .filter(|d| valid_rect(d))
        .map(|d| {
            point(
                d.origin.x + (d.size.width - default.width) / 2.,
                d.origin.y + (d.size.height - default.height) / 2.,
            )
        })
        .unwrap_or_else(|| point(px(0.), px(0.)));
    Bounds {
        origin,
        size: default,
    }
}
