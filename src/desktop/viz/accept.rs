use super::super::{motion, theme};
use super::data::{
    BinaryStar, Bounded, ChartKind, ChartSpec, ChartValue, Filtered, FunnelStage, OrbitMember,
    OrbitState, Selection, Series, StarPhase, TimelineItem, Tone,
};
use super::elements::{self, item_id, phase, series_id, stage_id, unit_interval};
use gpui_kit::{Element, ElementId, IntoElement};
use std::prelude::v1::test;
use std::time::Instant;

#[test]
fn chart_unknown_is_a_gap_with_a_marker_not_a_zero() {
    let mut series = Series::new(5);
    series.push(1, 3.);
    series.push(2, 4.);
    series.push_value(3, ChartValue::Unknown);
    series.push(4, 6.);
    series.push(5, 7.);
    assert_eq!(series.count(), 4);
    assert_eq!(series.sum(), 20.);
    assert_eq!(
        series
            .iter_points()
            .map(|(k, v)| (k, v))
            .collect::<Vec<_>>(),
        vec![
            (1, ChartValue::Known(3.)),
            (2, ChartValue::Known(4.)),
            (3, ChartValue::Unknown),
            (4, ChartValue::Known(6.)),
            (5, ChartValue::Known(7.)),
        ]
    );
    let samples = elements::chart_samples(&series);
    assert_eq!(
        samples.segments,
        vec![vec![(1, 3.), (2, 4.)], vec![(4, 6.), (5, 7.)]]
    );
    assert_eq!(samples.unknown, vec![3]);
    assert_eq!(elements::chart_bounds(&[samples]), Some((1, 5, 0., 7.)));
    assert_eq!(
        elements::unknown_marker_color(&theme::night()),
        theme::night().muted
    );

    series.push_value(5, ChartValue::Unknown);
    series.push(3, 9.);
    assert_eq!(series.sum(), 22.);
    series.push_value(6, ChartValue::Unknown);
    assert_eq!(
        series.iter_points().map(|(k, _)| k).collect::<Vec<_>>(),
        vec![2, 3, 4, 5, 6]
    );
    assert_eq!(series.sum(), 19.);
    let mut unknown_only = Series::new(2);
    unknown_only.push_value(12, ChartValue::Unknown);
    let samples = elements::chart_samples(&unknown_only);
    assert!(samples.segments.is_empty());
    assert_eq!(samples.unknown, vec![12]);
    assert_eq!(elements::chart_bounds(&[samples]), Some((12, 12, 0., 0.)));

    unknown_only.push(13, 0.);
    unknown_only.push(14, 2.); // Evicts the unknown, not the known zero.
    unknown_only.push_value(11, ChartValue::Unknown); // Too old for the window.
    assert_eq!(
        unknown_only.iter_points().collect::<Vec<_>>(),
        vec![(13, ChartValue::Known(0.)), (14, ChartValue::Known(2.)),]
    );
    assert!(elements::chart_samples(&unknown_only).unknown.is_empty());
    let mut zero_capacity = Series::new(0);
    zero_capacity.push_value(1, ChartValue::Unknown);
    assert_eq!(zero_capacity.iter_points().count(), 0);
}

fn check(series: &Series, values: &[f64]) {
    let sum: f64 = values.iter().sum();
    assert!((series.sum() - sum).abs() < 1e-8);
    assert_eq!(series.count(), values.len());
    assert_eq!(series.min(), values.iter().copied().reduce(f64::min));
    assert_eq!(series.max(), values.iter().copied().reduce(f64::max));
    assert_eq!(
        series.mean(),
        if values.is_empty() {
            None
        } else {
            Some(sum / values.len() as f64)
        }
    );
}

#[test]
fn bounded_two_hundred_thousand() {
    let mut window = Bounded::new(127);
    for n in 0..200_000 {
        window.insert(n, n);
    }
    assert_eq!(window.len(), 127);
    assert_eq!(window.iter().next().map(|(k, _)| *k), Some(199_873));
    assert_eq!(window.get(&199_999), Some(&199_999));
    let mut empty = Bounded::new(0);
    empty.insert(1, 1);
    assert_eq!(empty.len(), 0);
}

#[test]
fn insertion_order_and_replacement() {
    let mut a = Bounded::new(500);
    let mut b = Bounded::new(500);
    for i in 0..1000 {
        a.insert(i, i);
    }
    for i in (0..1000).rev() {
        b.insert(i, i);
    }
    a.insert(999, 4);
    b.insert(999, 4);
    assert_eq!(
        a.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
        b.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>()
    );
}

// Integer arithmetic is an exact oracle for the generated values and window size.
fn oracle(
    values: impl IntoIterator<Item = i64>,
) -> (usize, f64, Option<f64>, Option<f64>, Option<f64>) {
    let (mut count, mut sum, mut min, mut max) = (0usize, 0i64, None::<i64>, None::<i64>);
    for value in values {
        count += 1;
        sum += value;
        min = Some(min.map_or(value, |v| v.min(value)));
        max = Some(max.map_or(value, |v| v.max(value)));
    }
    (
        count,
        sum as f64,
        min.map(|v| v as f64),
        max.map(|v| v as f64),
        (count != 0).then(|| sum as f64 / count as f64),
    )
}

#[test]
fn seeded_aggregates_and_filter_switches() {
    let mut series = Series::new(73);
    // Independent fixed-domain model: replacement and smallest-key eviction are explicit.
    let mut model = [None::<i64>; 300];
    let mut seed = 489123u64;
    for _ in 0..10_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let ts = (seed % 300) as usize;
        let value = ((seed >> 16) % 2001) as i64 - 1000;
        series.push(ts as i64, value as f64);
        model[ts] = Some(value);
        if model.iter().flatten().count() > 73 {
            let first = model.iter().position(Option::is_some).unwrap();
            model[first] = None;
        }
        let all = oracle(model.iter().flatten().copied());
        assert_eq!(
            (
                series.count(),
                series.sum(),
                series.min(),
                series.max(),
                series.mean()
            ),
            all
        );

        let divisor = (seed % 7 + 1) as usize;
        let filtered = Filtered::new(series.window(), |k, _| *k % divisor as i64 == 0);
        let expected = oracle(model.iter().enumerate().filter_map(|(k, value)| {
            if k % divisor == 0 {
                *value
            } else {
                None
            }
        }));
        assert_eq!(
            (
                filtered.count(),
                filtered.sum(),
                filtered.min(),
                filtered.max(),
                filtered.mean()
            ),
            expected
        );

        // Rebuild the selected Series incrementally each time the filter changes.
        let mut selected = Series::new(73);
        for (key, value) in filtered.iter() {
            selected.push(*key, *value);
        }
        assert_eq!(
            (
                selected.count(),
                selected.sum(),
                selected.min(),
                selected.max(),
                selected.mean()
            ),
            expected
        );
    }
}

#[test]
fn element_helpers_handle_motion_values_and_item_ids() {
    let now = Instant::now();
    let mut inactive = motion::Driver::at(now, false);
    assert_eq!(phase(&mut inactive, false), 0.);
    assert!(!inactive.wants_frame());
    let mut reduced = motion::Driver::at(now, true);
    assert_eq!(phase(&mut reduced, true), 0.);
    assert!(!reduced.wants_frame());
    let mut active = motion::Driver::at(now, false);
    phase(&mut active, true);
    assert!(active.wants_frame());

    for (input, expected) in [
        (-1., 0.),
        (0., 0.),
        (0.4, 0.4),
        (1., 1.),
        (2., 1.),
        (f32::NAN, 0.),
        (f32::INFINITY, 0.),
        (f32::NEG_INFINITY, 0.),
    ] {
        assert_eq!(unit_interval(input), expected);
    }
    assert_eq!(item_id("member-a"), ElementId::from("member-a"));
    assert_ne!(item_id("member-a"), item_id("member-b"));
    assert_eq!(stage_id(2), ElementId::from(("funnel-stage", 2usize)));
    assert_ne!(stage_id(2), stage_id(3));
    assert_eq!(series_id(2), ElementId::from(("chart-series", 2usize)));
    assert_ne!(series_id(2), series_id(3));
}

#[test]
fn elements_preserve_root_ids_and_schedule_motion() {
    let tokens = theme::night();
    let density = theme::Density::Comfortable;
    let timeline_items = [TimelineItem {
        id: "event".into(),
        at_ms: 1,
        label: "event".into(),
        tone: Tone::Good,
    }];
    let stages = [FunnelStage {
        label: "stage".into(),
        count: 1,
    }];
    let mut members = [OrbitMember {
        id: "member".into(),
        label: "member".into(),
        state: OrbitState::Idle,
        weight: 0.5,
    }];
    let mut star = BinaryStar {
        a_label: "a".into(),
        b_label: "b".into(),
        phase: StarPhase::Apart,
        agreement: 0.5,
    };
    let spec = ChartSpec {
        kind: ChartKind::Line,
        series: vec![("series".into(), Series::new(2))],
    };
    assert_eq!(
        elements::timeline(
            "timeline-root",
            &timeline_items,
            &Selection::new(Some("event".into())),
            &tokens,
            density
        )
        .into_element()
        .id(),
        Some("timeline-root".into())
    );
    assert_eq!(
        elements::funnel("funnel-root", &stages, &tokens, density)
            .into_element()
            .id(),
        Some("funnel-root".into())
    );
    assert_eq!(
        elements::chart("chart-root", &spec, &tokens, density)
            .into_element()
            .id(),
        Some("chart-root".into())
    );
    let mut driver = motion::Driver::at(Instant::now(), false);
    assert_eq!(
        elements::orbit("orbit-root", &members, &mut driver, &tokens, density)
            .into_element()
            .id(),
        Some("orbit-root".into())
    );
    assert!(!driver.wants_frame());
    members[0].state = OrbitState::Speaking;
    let mut driver = motion::Driver::at(Instant::now(), false);
    let _ = elements::orbit("orbit-root", &members, &mut driver, &tokens, density);
    assert!(driver.wants_frame());
    let mut driver = motion::Driver::at(Instant::now(), false);
    assert_eq!(
        elements::binary_star("star-root", &star, &mut driver, &tokens, density)
            .into_element()
            .id(),
        Some("star-root".into())
    );
    assert!(!driver.wants_frame());
    star.phase = StarPhase::Approaching;
    let mut driver = motion::Driver::at(Instant::now(), false);
    let _ = elements::binary_star("star-root", &star, &mut driver, &tokens, density);
    assert!(driver.wants_frame());
}

#[test]
fn selection_survives_until_evicted_or_filtered() {
    let mut window = Bounded::new(3);
    for n in 1..=2 {
        window.insert(n, n);
    }
    let mut selection = Selection::new(Some(2));
    window.insert(3, 3);
    assert_eq!(selection.reconcile(&window, |_, _| true), None);
    assert_eq!(selection.key(), Some(&2));
    window.insert(4, 4);
    assert_eq!(selection.reconcile(&window, |k, _| *k != 3), None);
    assert_eq!(selection.key(), Some(&2));
    assert!(selection.reconcile(&window, |k, _| *k != 2).is_some());
    assert_eq!(selection.key(), None);
    assert_eq!(selection.reconcile(&window, |_, _| false), None);
    selection.select(Some(2));
    window.insert(5, 5);
    assert!(selection.reconcile(&window, |_, _| true).is_some());
    assert_eq!(selection.key(), None);
}

#[test]
fn fifty_thousand_row_regression() {
    let mut s = Series::new(50_000);
    for n in 0..50_000 {
        s.push(n, (n % 13) as f64);
    }
    check(&s, &s.iter().map(|(_, v)| *v).collect::<Vec<_>>());
}

#[test]
fn brute_force_comparison_has_a_negative_control() {
    let mut s = Series::new(10);
    s.push(1, 4.);
    s.push(2, 8.);
    let naive: f64 = s.iter().map(|(_, v)| *v).sum();
    assert_eq!(s.sum(), naive);
    s.corrupt_sum_for_test();
    assert_ne!(s.sum(), naive);
}

#[test]
fn filtered_accepts_non_numeric_items() {
    let mut items = Bounded::new(2);
    items.insert(1, String::from("keep"));
    items.insert(2, String::from("skip"));
    let view = Filtered::new(&items, |_, value| value == "keep");
    assert_eq!(
        view.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn cancellation_and_nonfinite_evictions() {
    let mut series = Series::new(2);
    series.push(1, 1e16);
    series.push(2, 1.);
    series.push(3, 2.);
    assert_eq!(series.sum(), 3.);
    let mut wide = Series::new(3);
    wide.push(1, 1e100);
    wide.push(2, 1e50);
    wide.push(3, 1.);
    wide.push(4, 2.);
    wide.push(5, 3.);
    assert_eq!(wide.sum(), 6.);
    let mut overflow = Series::new(3);
    overflow.push(1, 1e308);
    overflow.push(2, 1e308);
    assert_eq!(overflow.sum(), f64::INFINITY);
    overflow.push(3, -1e308);
    assert_eq!(overflow.sum(), 1e308);
    overflow.push(4, 1.);
    assert_eq!(overflow.sum(), 1.);
    let mut tiny = Series::new(3);
    tiny.push(1, f64::from_bits(1));
    tiny.push(2, f64::from_bits(1));
    tiny.push(3, -f64::from_bits(1));
    assert_eq!(tiny.sum(), f64::from_bits(1));
    let mut round = Series::new(3);
    round.push(1, 1.);
    round.push(2, f64::EPSILON / 2.);
    assert_eq!(round.sum(), 1.);
    round.push(3, f64::EPSILON);
    assert_eq!(round.sum(), f64::from_bits(1f64.to_bits() + 2));
    series.push(4, f64::INFINITY);
    assert_eq!(series.sum(), f64::INFINITY);
    series.push(5, 5.);
    series.push(6, 6.);
    assert_eq!(series.sum(), 11.);
}

fn has_forbidden_import(tree: &syn::UseTree, forbidden: &[String]) -> bool {
    match tree {
        syn::UseTree::Path(path) => {
            forbidden.iter().any(|name| *name == path.ident.to_string())
                || has_forbidden_import(&path.tree, forbidden)
        }
        syn::UseTree::Name(name) => forbidden.iter().any(|word| *word == name.ident.to_string()),
        syn::UseTree::Rename(rename) => forbidden
            .iter()
            .any(|word| *word == rename.ident.to_string()),
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| has_forbidden_import(tree, forbidden)),
        syn::UseTree::Glob(_) => false,
    }
}

#[test]
fn source_uses_only_foundations_and_no_hex_colours() {
    for source in [
        include_str!("mod.rs"),
        include_str!("data.rs"),
        include_str!("elements.rs"),
        include_str!("accept.rs"),
    ] {
        struct Imports<'a>(&'a [String]);
        impl<'ast> syn::visit::Visit<'ast> for Imports<'_> {
            fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
                assert!(
                    !has_forbidden_import(&item.tree, self.0),
                    "forbidden import"
                );
            }
        }
        let forbidden = [
            ["eng", "ine"].concat(),
            ["li", "ve"].concat(),
            ["adapt", "ers"].concat(),
        ];
        let parsed = syn::parse_file(source).expect("valid Rust source");
        syn::visit::Visit::visit_file(&mut Imports(&forbidden), &parsed);
        let bytes = source.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == 35 {
                let digits = bytes[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_hexdigit())
                    .count();
                assert!(![3, 4, 6, 8].contains(&digits), "hex colour literal");
            }
        }
    }
}
