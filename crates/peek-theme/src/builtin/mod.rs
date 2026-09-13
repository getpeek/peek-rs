//! The six built-in themes. Adding a `ThemeId` variant fails to compile until a table exists.

mod blueprint;
mod midday;
mod midnight;
mod paper;
mod pine;
mod terminal;

use peek_config::ThemeId;

use crate::spec::ThemeSpec;

#[must_use]
pub fn spec(id: ThemeId) -> &'static ThemeSpec {
    match id {
        ThemeId::Pine => &pine::PINE,
        ThemeId::Midnight => &midnight::MIDNIGHT,
        ThemeId::Midday => &midday::MIDDAY,
        ThemeId::Terminal => &terminal::TERMINAL,
        ThemeId::Paper => &paper::PAPER,
        ThemeId::Blueprint => &blueprint::BLUEPRINT,
    }
}

/// Every built-in, in picker order.
pub fn all() -> impl Iterator<Item = &'static ThemeSpec> {
    ThemeId::ALL.iter().map(|id| spec(*id))
}
