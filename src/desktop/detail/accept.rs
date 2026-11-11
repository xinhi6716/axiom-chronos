use super::TABLE;
use std::prelude::v1::test;

fn production() -> &'static str {
    let source = include_str!("../detail.rs");
    let end = source.find("#[cfg(test)]").expect("test boundary");
    assert!(end > source.find("fn render").expect("render"));
    &source[..end]
}

fn literal_size_or_radius_hits(source: &str) -> usize {
    ["text_size(px(", "rounded(px(", ".px(px(", ".gap(px(", ".mt(px("].iter().map(|pattern| source.matches(pattern).count()).sum()
}

#[test]
fn theme_sizes_and_radii_have_no_pixel_literals() {
    assert_eq!(literal_size_or_radius_hits(production()), 0);
    assert_eq!(literal_size_or_radius_hits(".text_size(px(13.)).rounded(px(4.)).px(px(1.)).gap(px(2.)).mt(px(3.))"), 5);
}

#[test]
fn every_button_has_a_tooltip() {
    use composer_super::buttons_without_tooltips;
    assert!(buttons_without_tooltips(production()).is_empty(), "buttons without tooltip at lines {:?}", buttons_without_tooltips(production()));
    assert_eq!(buttons_without_tooltips("Button::new(\"missing\").ghost()"), vec![1]);
    assert_eq!(buttons_without_tooltips("Button::new(\"missing\"); Button::new(\"ok\").tooltip(\"tip\")"), vec![1]);
    assert_eq!(buttons_without_tooltips("Button::new(\"ok\").tooltip(\"tip\"); Button::new(\"missing\"); div().tooltip(\"unrelated\")"), vec![1]);
}

#[test]
fn stable_ids_and_clipboard_tooltip_are_present() {
    let source = production();
    for id in ["detail-back", "unit-module", "doc-copy", "doc-open", "doc-open-full", "need-resume", "unit-row", "module-row"] {
        assert!(source.contains(&format!("\"{id}\"")), "missing id {id}");
    }
    let start = source.find("Clipboard::new((\"doc-copy\"").expect("doc-copy clipboard");
    assert!(composer_super::builder_expression(&source[start..]).contains(".tooltip("));
}

#[test]
fn comment_button_is_localized() {
    let source = production();
    assert!(source.contains("Button::new(\"detail-comment\")"));
    assert!(source.contains("Comment on this item"));
    for (page, next) in [("    fn unit_page(", "    fn tab_body("), ("    fn module_page(", "    fn contract_section(")] {
        let body = source.split_once(page).expect("page").1.split_once(next).expect("page end").0;
        assert!(body.contains("comment_button("), "{page} must call comment_button");
    }
    assert!(source.contains("address_interjection"));
    assert!(source.contains("subject_of"));
    assert!(TABLE.iter().any(|(en, zh)| *en == "Comment on this item" && !zh.is_empty()));
}

mod composer_super {
    use std::prelude::v1::test;

    fn production(source: &str) -> &str {
        source.split_once("#[cfg(test)]").expect("test boundary").0
    }

    fn literal_hits(source: &str) -> usize {
        ["text_size(px(", "rounded(px("].iter().map(|pattern| source.matches(pattern).count()).sum()
    }

    // Stop at the end of this builder expression, not at another element's tooltip.
    pub(super) fn builder_expression(source: &str) -> &str {
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        let mut end = source.len();
        for (offset, ch) in source.char_indices() {
            if quoted {
                if escaped { escaped = false; }
                else if ch == '\\' { escaped = true; }
                else if ch == '"' { quoted = false; }
                continue;
            }
            match ch {
                '"' => quoted = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                ')' | ']' | '}' | ',' | ';' if depth == 0 => { end = offset; break; }
                _ => {}
            }
        }
        &source[..end]
    }

    pub(super) fn buttons_without_tooltips(source: &str) -> Vec<usize> {
        source.match_indices("Button::new(").filter_map(|(start, _)| {
            let builder = builder_expression(&source[start..]);
            (!builder.contains(".tooltip(") && !builder.contains(".tooltip_with_action("))
                .then_some(source[..start].bytes().filter(|&byte| byte == b'\n').count() + 1)
        }).collect()
    }

    #[test]
    fn composer_and_switch_use_theme_sizes_and_radii() {
        for source in [include_str!("../composer.rs"), include_str!("../superswitch.rs")] {
            assert_eq!(literal_hits(production(source)), 0);
        }
        assert_eq!(literal_hits(".text_size(px(13.)).rounded(px(4.))"), 2);
    }

    #[test]
    fn composer_buttons_have_their_own_tooltips() {
        let source = production(include_str!("../composer.rs"));
        assert_eq!(source.matches("Button::new(").count(), 6);
        assert_eq!(buttons_without_tooltips(source), Vec::<usize>::new());
        assert_eq!(buttons_without_tooltips("Button::new(\"missing\"); Button::new(\"ok\").tooltip(\"tip\")"), vec![1]);
        assert_eq!(buttons_without_tooltips("Button::new(\"ok\").tooltip(\"tip\"); Button::new(\"missing\")"), vec![1]);
    }

    #[test]
    fn sync_restores_the_invoker() {
        let source = production(include_str!("../composer.rs"));
        let sync = source.split_once("    fn sync(").expect("sync").1.split_once("    fn reset(").expect("sync end").0;
        assert!(sync.contains("invoker"));
        assert!(sync.contains(".focus(window, cx)"));
    }
}
