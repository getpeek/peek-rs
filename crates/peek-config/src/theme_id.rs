use serde::{Deserialize, Serialize};

/// The built-in themes, persisted lowercase as `"theme": "pine"`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeId {
    #[default]
    Pine,
    Midnight,
    Midday,
    Terminal,
    Paper,
    Blueprint,
}

impl ThemeId {
    pub const ALL: [Self; 6] = [
        Self::Pine,
        Self::Midnight,
        Self::Midday,
        Self::Terminal,
        Self::Paper,
        Self::Blueprint,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pine => "pine",
            Self::Midnight => "midnight",
            Self::Midday => "midday",
            Self::Terminal => "terminal",
            Self::Paper => "paper",
            Self::Blueprint => "blueprint",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_variant_lowercase() {
        for theme in ThemeId::ALL {
            let json = serde_json::to_string(&theme).unwrap();
            assert_eq!(json, format!("\"{}\"", theme.as_str()));
            assert_eq!(serde_json::from_str::<ThemeId>(&json).unwrap(), theme);
        }
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(serde_json::from_str::<ThemeId>("\"neon\"").is_err());
    }
}
