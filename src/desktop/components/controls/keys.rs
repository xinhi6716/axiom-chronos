//! Shared keyboard actions and bindings for components.

use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(
    components,
    [Confirm, Cancel, MoveUp, MoveDown, PageUp, PageDown, First, Last]
);

pub const LIST_BINDINGS: &[(&str, &str)] = &[
    ("enter", "Confirm"),
    ("escape", "Cancel"),
    ("up", "MoveUp"),
    ("down", "MoveDown"),
    ("pageup", "PageUp"),
    ("pagedown", "PageDown"),
    ("home", "First"),
    ("end", "Last"),
];

pub const FIELD_BINDINGS: &[(&str, &str)] = &[("enter", "Confirm"), ("escape", "Cancel")];

fn bindings(
    table: &'static [(&'static str, &'static str)],
    context: &'static str,
) -> impl Iterator<Item = KeyBinding> {
    table.iter().map(move |&(key, action)| match action {
        "Confirm" => KeyBinding::new(key, Confirm, Some(context)),
        "Cancel" => KeyBinding::new(key, Cancel, Some(context)),
        "MoveUp" => KeyBinding::new(key, MoveUp, Some(context)),
        "MoveDown" => KeyBinding::new(key, MoveDown, Some(context)),
        "PageUp" => KeyBinding::new(key, PageUp, Some(context)),
        "PageDown" => KeyBinding::new(key, PageDown, Some(context)),
        "First" => KeyBinding::new(key, First, Some(context)),
        "Last" => KeyBinding::new(key, Last, Some(context)),
        _ => unreachable!("unknown component action: {action}"),
    })
}

pub fn bind_list(context: &'static str, cx: &mut App) {
    cx.bind_keys(bindings(LIST_BINDINGS, context));
}

pub fn bind_field(context: &'static str, cx: &mut App) {
    cx.bind_keys(bindings(FIELD_BINDINGS, context));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Action, Keystroke};
    use std::collections::HashSet;

    #[test]
    fn action_names_are_stable_and_distinct() {
        let names = [
            Confirm.name(),
            Cancel.name(),
            MoveUp.name(),
            MoveDown.name(),
            PageUp.name(),
            PageDown.name(),
            First.name(),
            Last.name(),
        ];
        assert_eq!(
            names,
            [
                "components::Confirm",
                "components::Cancel",
                "components::MoveUp",
                "components::MoveDown",
                "components::PageUp",
                "components::PageDown",
                "components::First",
                "components::Last",
            ]
        );
        assert_eq!(
            names.iter().copied().collect::<HashSet<_>>().len(),
            names.len()
        );
    }

    #[test]
    fn binding_tables_and_generated_bindings_are_exact() {
        assert_eq!(
            LIST_BINDINGS,
            &[
                ("enter", "Confirm"),
                ("escape", "Cancel"),
                ("up", "MoveUp"),
                ("down", "MoveDown"),
                ("pageup", "PageUp"),
                ("pagedown", "PageDown"),
                ("home", "First"),
                ("end", "Last"),
            ]
        );
        assert_eq!(
            FIELD_BINDINGS,
            &[("enter", "Confirm"), ("escape", "Cancel")]
        );
        for table in [LIST_BINDINGS, FIELD_BINDINGS] {
            let actual: Vec<_> = bindings(table, "TestContext").collect();
            assert_eq!(actual.len(), table.len());
            for (binding, (key, action)) in actual.iter().zip(table) {
                let expected = Keystroke::parse(key).unwrap();
                assert_eq!(binding.keystrokes().len(), 1);
                assert_eq!(binding.keystrokes()[0].inner(), &expected);
                assert_eq!(binding.action().name(), format!("components::{action}"));
            }
        }
    }
}
