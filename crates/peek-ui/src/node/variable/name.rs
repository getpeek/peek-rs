//! Why a variable row's name would not reach a query. The grammar itself lives in
//! `peek_document::is_variable_name`, beside the `VariableRow` it constrains, because the
//! canvas tools validate against it too.

use peek_document::{VariableRow, is_variable_name};

/// Why a row's name would not reach a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameProblem {
    Malformed,
    Duplicate,
}

impl NameProblem {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Malformed => {
                "Starts with a letter or underscore, then letters, digits or underscores"
            }
            Self::Duplicate => "Another row already uses this name",
        }
    }
}

/// An empty name is unfinished rather than wrong — the reference only flags what the user has
/// actually typed, so a node of blank rows shows no errors.
pub(super) fn problem(name: &str, rows: &[VariableRow]) -> Option<NameProblem> {
    if name.is_empty() {
        return None;
    }
    if !is_variable_name(name) {
        return Some(NameProblem::Malformed);
    }
    if rows.iter().filter(|row| row.name == name).count() > 1 {
        return Some(NameProblem::Duplicate);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{NameProblem, problem};
    use peek_document::{VariableRow, VariableValue};

    fn rows(names: &[&str]) -> Vec<VariableRow> {
        names
            .iter()
            .map(|name| VariableRow {
                name: (*name).to_string(),
                value: VariableValue::One(String::new()),
            })
            .collect()
    }

    #[test]
    fn a_blank_name_is_not_a_problem() {
        assert_eq!(problem("", &rows(&["", ""])), None);
    }

    #[test]
    fn names_follow_the_reference_grammar() {
        let rows = rows(&[]);
        assert_eq!(problem("user_id", &rows), None);
        assert_eq!(problem("_id2", &rows), None);
        assert_eq!(problem("2id", &rows), Some(NameProblem::Malformed));
        assert_eq!(problem("user id", &rows), Some(NameProblem::Malformed));
        assert_eq!(problem("user-id", &rows), Some(NameProblem::Malformed));
        assert_eq!(problem("användare", &rows), Some(NameProblem::Malformed));
    }

    #[test]
    fn a_name_claimed_twice_is_a_duplicate() {
        let rows = rows(&["limit", "limit", "offset"]);
        assert_eq!(problem("limit", &rows), Some(NameProblem::Duplicate));
        assert_eq!(problem("offset", &rows), None);
    }
}
