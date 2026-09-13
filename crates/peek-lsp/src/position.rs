use lsp_types::Position;

/// Convert an LSP position (line + UTF-16 code-unit character) to a byte offset
/// in the source string. Returns `None` if the position is past the end of the
/// document.
#[must_use]
pub(crate) fn position_to_byte_offset(source: &str, pos: Position) -> Option<usize> {
    let line_start = if pos.line == 0 {
        0
    } else {
        let mut newlines = 0u32;
        let mut start = None;
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                newlines += 1;
                if newlines == pos.line {
                    start = Some(i + 1);
                    break;
                }
            }
        }
        start?
    };

    let after_line_start = &source[line_start..];
    let line_text = after_line_start
        .split_once('\n')
        .map_or(after_line_start, |(l, _)| l);

    let mut utf16_units = 0u32;
    for (byte_idx, c) in line_text.char_indices() {
        if utf16_units == pos.character {
            return Some(line_start + byte_idx);
        }
        utf16_units = utf16_units.saturating_add(u32::from(u8::try_from(c.len_utf16()).ok()?));
    }
    if utf16_units == pos.character {
        return Some(line_start + line_text.len());
    }
    None
}

/// Convert a byte offset to an LSP position, counting the column in **characters**.
///
/// The editor host reads `Position::character` as a character count rather than the
/// spec's UTF-16 code units, so a range produced here lands where it was meant to;
/// [`position_to_byte_offset`] answers the spec form, for callers that send one.
#[must_use]
pub(crate) fn byte_offset_to_position(source: &str, offset: usize) -> Position {
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        character: u32::try_from(before[line_start..].chars().count()).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn start_of_first_line_is_zero() {
        assert_eq!(position_to_byte_offset("hello", pos(0, 0)), Some(0));
    }

    #[test]
    fn ascii_within_first_line() {
        assert_eq!(position_to_byte_offset("hello", pos(0, 3)), Some(3));
    }

    #[test]
    fn end_of_line_is_line_length() {
        assert_eq!(position_to_byte_offset("hello", pos(0, 5)), Some(5));
    }

    #[test]
    fn second_line_after_lf() {
        assert_eq!(position_to_byte_offset("foo\nbar", pos(1, 0)), Some(4));
        assert_eq!(position_to_byte_offset("foo\nbar", pos(1, 2)), Some(6));
    }

    #[test]
    fn beyond_end_returns_none() {
        assert_eq!(position_to_byte_offset("hi", pos(0, 5)), None);
        assert_eq!(position_to_byte_offset("hi", pos(2, 0)), None);
    }
}
