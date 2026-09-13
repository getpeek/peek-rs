//! Translation of Peek's keymap syntax (`docs/keymap.md`) into gpui keystrokes.
//!
//! A Peek combo is a lowercase trigger optionally prefixed by `meta`/`shift`/`alt`/`ctrl`
//! joined with `-`, in any order. gpui wants `cmd` and arrow keys as `left`/`right`/…; we
//! also emit modifiers in one fixed order (Zed's `ctrl-alt-cmd-shift`) so that two spellings
//! of one combo compare equal — which is what makes merge-by-key over the user's overrides
//! correct.

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum KeymapError {
    UnknownModifier(String),
    DuplicateModifier(String),
    EmptyKey,
}

impl fmt::Display for KeymapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownModifier(modifier) => write!(formatter, "unknown modifier {modifier:?}"),
            Self::DuplicateModifier(modifier) => {
                write!(formatter, "modifier {modifier:?} given twice")
            }
            Self::EmptyKey => write!(formatter, "combo has no trigger key"),
        }
    }
}

impl std::error::Error for KeymapError {}

/// Converts `"meta-shift-0"` to `"cmd-)"`, `"meta-arrowleft"` to `"cmd-left"`,
/// `"shift-meta-z"` to `"cmd-shift-z"` — the string `gpui::KeyBinding::new` accepts.
///
/// # Errors
/// Returns an error for unknown or repeated modifiers and for a missing trigger key.
pub fn gpui_keystroke(combo: &str) -> Result<String, KeymapError> {
    let combo = combo.trim().to_ascii_lowercase();
    let (modifier_part, key) = split_trigger(&combo);
    if key.is_empty() {
        return Err(KeymapError::EmptyKey);
    }

    let mut modifiers = Modifiers::default();
    for modifier in modifier_part.split('-').filter(|part| !part.is_empty()) {
        modifiers.add(modifier)?;
    }

    let key = translate_key(key);
    let key = match shifted(key) {
        Some(shifted) if modifiers.shift => {
            modifiers.shift = false;
            shifted
        }
        _ => key,
    };

    Ok(format!("{}{}", modifiers.prefix(), key))
}

/// Splits off the trigger key. A doubled trailing `-` (`"meta--"`) or a lone `-` means the
/// key itself is `-`; a single trailing `-` (`"meta-"`) is a missing key.
fn split_trigger(combo: &str) -> (&str, &str) {
    match combo.rsplit_once('-') {
        Some(("", "")) => ("", "-"),
        Some((head, "")) => match head.strip_suffix('-') {
            Some(modifiers) => (modifiers, "-"),
            None => (head, ""),
        },
        Some((head, key)) => (head, key),
        None => ("", combo),
    }
}

// Mirrors gpui's `Modifiers`, which is also four flags.
#[allow(clippy::struct_excessive_bools, reason = "mirrors gpui::Modifiers")]
#[derive(Default)]
struct Modifiers {
    control: bool,
    alt: bool,
    shift: bool,
    command: bool,
}

impl Modifiers {
    fn add(&mut self, modifier: &str) -> Result<(), KeymapError> {
        let slot = match modifier {
            "ctrl" | "control" => &mut self.control,
            "alt" | "option" => &mut self.alt,
            "shift" => &mut self.shift,
            "meta" | "cmd" | "command" => &mut self.command,
            other => return Err(KeymapError::UnknownModifier(other.to_string())),
        };
        if *slot {
            return Err(KeymapError::DuplicateModifier(modifier.to_string()));
        }
        *slot = true;
        Ok(())
    }

    fn prefix(&self) -> String {
        let mut prefix = String::new();
        // Zed's canonical spelling: ctrl, alt, cmd, shift.
        for (enabled, name) in [
            (self.control, "ctrl-"),
            (self.alt, "alt-"),
            (self.command, "cmd-"),
            (self.shift, "shift-"),
        ] {
            if enabled {
                prefix.push_str(name);
            }
        }
        prefix
    }
}

/// The character a shifted key types on a US layout, for the keys where macOS reports the
/// shifted character and clears the shift flag rather than reporting shift plus the base key
/// (`gpui-pre-macos`'s `parse_keystroke`: shift survives only when the unmodified key is all
/// ASCII lowercase). A binding spelled `cmd-shift-0` would therefore never match the `cmd-)`
/// the window actually delivers. Named keys (`left`, `tab`, `f1`) keep their shift and are
/// absent here.
fn shifted(key: &str) -> Option<&'static str> {
    let shifted = match key {
        "1" => "!",
        "2" => "@",
        "3" => "#",
        "4" => "$",
        "5" => "%",
        "6" => "^",
        "7" => "&",
        "8" => "*",
        "9" => "(",
        "0" => ")",
        "-" => "_",
        "=" => "+",
        "[" => "{",
        "]" => "}",
        "\\" => "|",
        ";" => ":",
        "'" => "\"",
        "," => "<",
        "." => ">",
        "/" => "?",
        "`" => "~",
        _ => return None,
    };
    Some(shifted)
}

fn translate_key(key: &str) -> &str {
    match key {
        "arrowleft" => "left",
        "arrowright" => "right",
        "arrowup" => "up",
        "arrowdown" => "down",
        "esc" => "escape",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_every_default_binding() {
        let cases = [
            ("escape", "escape"),
            ("l", "l"),
            ("meta-x", "cmd-x"),
            ("backspace", "backspace"),
            ("shift-meta-z", "cmd-shift-z"),
            ("meta-shift-z", "cmd-shift-z"),
            ("meta-0", "cmd-0"),
            ("meta-shift-0", "cmd-)"),
            ("meta-shift-[", "cmd-{"),
            ("meta-]", "cmd-]"),
            ("meta-arrowleft", "cmd-left"),
            ("meta-arrowdown", "cmd-down"),
            ("meta-.", "cmd-."),
            ("meta-/", "cmd-/"),
            ("shift-p", "shift-p"),
            ("ctrl-alt-shift-meta-k", "ctrl-alt-cmd-shift-k"),
            ("META-Shift-A", "cmd-shift-a"),
        ];
        for (peek, gpui) in cases {
            assert_eq!(gpui_keystroke(peek).unwrap(), gpui, "{peek}");
        }
    }

    /// macOS clears the shift flag for these, so a `shift-` spelling would never match
    /// (`shifted`). Letters keep theirs, which `translates_every_default_binding` covers.
    #[test]
    fn shifted_non_letters_become_the_character_they_type() {
        let cases = [
            ("meta-shift-]", "cmd-}"),
            ("shift-1", "!"),
            ("meta-shift-.", "cmd->"),
            ("meta-shift--", "cmd-_"),
            ("shift-tab", "shift-tab"),
            ("meta-shift-arrowleft", "cmd-shift-left"),
        ];
        for (peek, gpui) in cases {
            assert_eq!(gpui_keystroke(peek).unwrap(), gpui, "{peek}");
        }
    }

    #[test]
    fn a_trailing_dash_is_the_minus_key() {
        assert_eq!(gpui_keystroke("meta--").unwrap(), "cmd--");
        assert_eq!(gpui_keystroke("-").unwrap(), "-");
    }

    #[test]
    fn rejects_bad_combos() {
        assert_eq!(
            gpui_keystroke("hyper-x"),
            Err(KeymapError::UnknownModifier("hyper".to_string()))
        );
        assert_eq!(
            gpui_keystroke("meta-meta-x"),
            Err(KeymapError::DuplicateModifier("meta".to_string()))
        );
        assert_eq!(gpui_keystroke(""), Err(KeymapError::EmptyKey));
        assert_eq!(gpui_keystroke("meta-"), Err(KeymapError::EmptyKey));
    }
}
