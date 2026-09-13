//! Resolves the effective keymap (registry defaults merged with the user's overrides by
//! normalized key) and installs it into gpui.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::rc::Rc;

use gpui_kit::{App, KeyBinding, KeyBindingContextPredicate};
use peek_config::gpui_keystroke;

use super::{COMMANDS, Command, find};

/// Defaults from the registry with the user's overrides applied by key. Unknown action ids
/// and unparsable combos are logged and skipped so one typo never drops the rest
/// (`docs/keymap.md`, "Merge rules").
///
/// # Panics
/// Only if a registry default key fails to translate, which the registry tests rule out.
#[must_use]
pub fn resolved<S: BuildHasher>(
    overrides: &HashMap<String, String, S>,
) -> BTreeMap<String, &'static Command> {
    let mut resolved = BTreeMap::new();
    for command in COMMANDS {
        for combo in command.default_keys {
            let key = gpui_keystroke(combo).expect("registry default keys are valid");
            resolved.insert(key, command);
        }
    }
    for (combo, id) in overrides {
        let (Ok(key), Some(command)) = (gpui_keystroke(combo), find(id)) else {
            log::warn!("peek: ignoring keymap entry {combo:?} -> {id:?}");
            continue;
        };
        resolved.insert(key, command);
    }
    resolved
}

/// Installs the resolved keymap. Call once after `gpui_kit::init` so ties with gpui-kit's own
/// bindings resolve in our favour.
///
/// # Panics
/// Only if a registry context predicate is malformed, which the registry tests rule out.
pub fn bind<S: BuildHasher>(overrides: &HashMap<String, String, S>, cx: &mut App) {
    let mapper = cx.keyboard_mapper().clone();
    let bindings = resolved(overrides).into_iter().map(|(key, command)| {
        let predicate = KeyBindingContextPredicate::parse(command.context)
            .expect("registry contexts are valid predicates");
        KeyBinding::load(
            &key,
            (command.build)(),
            Some(Rc::new(predicate)),
            false,
            None,
            mapper.as_ref(),
        )
        .expect("resolved keys are parsable keystrokes")
    });
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_replace_by_key_and_keep_other_keys() {
        let overrides = HashMap::from([
            ("shift-meta-p".to_string(), "Zoom::FitView".to_string()),
            ("meta-9".to_string(), "Nope::Nothing".to_string()),
            ("bad--combo-".to_string(), "Zoom::In".to_string()),
        ]);
        let resolved = resolved(&overrides);
        assert_eq!(resolved["cmd-shift-p"].id, "Zoom::FitView");
        assert_eq!(resolved["cmd-p"].id, "CommandPalette::Open");
        assert_eq!(resolved["cmd-)"].id, "Zoom::FitView");
        assert!(!resolved.contains_key("cmd-9"));
    }
}
