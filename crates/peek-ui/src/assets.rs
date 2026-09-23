//! The application's icon source.
//!
//! `gpui_kit::assets::Assets` embeds the 101 icons the components themselves use, which does
//! not include the toolbar's tool glyphs. The full `AllAssets` catalog is 1830 SVGs and 7.4 MB,
//! so instead we embed exactly the icons Peek names and fall through to the default bundle for
//! everything the components ask for.

use std::borrow::Cow;

use gpui_kit::assets::icon_assets;
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    PeekIcons,
    [
        MousePointer2,
        Code,
        Sparkles,
        Type,
        AtSign,
        Pencil,
        Trash,
        Brackets,
        Lock,
        LockOpen,
        // The regions picker's trigger, in the zoom cluster.
        Map,
        ChevronDown,
        // The agent node.
        ChevronRight,
        GitFork,
        SendHorizontal,
        Square,
        Lightbulb,
        DatabaseZap,
        Bot,
        TriangleAlert,
        CircleCheck,
        CircleDot,
        Circle,
        Wrench,
        LoaderCircle,
        ShieldAlert,
        // The connection picker.
        Terminal,
        Key,
        X,
        // The result node's toolbar and export menu.
        ChartColumn,
        Download,
        Copy,
        Search,
        Rows3,
        Braces,
        Table,
        Database,
        // The query node's format button and the chart node's type picker.
        ListIndentIncrease,
        ChartLine,
        ChartArea,
        // The version-history timeline.
        Clock,
        Hand,
        RotateCcw,
    ]
);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = PeekIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = PeekIcons.list(path)?;
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        paths.sort_unstable();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::AssetSource;
    use gpui_kit::assets::IconName;

    /// An icon whose embedded name and whose `IconName` disagree renders as **nothing**, with no
    /// error anywhere — so every glyph Peek names is asserted to actually load.
    #[test]
    fn every_named_icon_resolves_to_bytes() {
        let named = [
            IconName::MousePointer2,
            IconName::Code,
            IconName::Sparkles,
            IconName::Type,
            IconName::AtSign,
            IconName::Pencil,
            IconName::Trash,
            IconName::Brackets,
            IconName::Lock,
            IconName::LockOpen,
            IconName::ChevronDown,
            IconName::ChevronRight,
            IconName::GitFork,
            IconName::SendHorizontal,
            IconName::Square,
            IconName::Lightbulb,
            IconName::DatabaseZap,
            IconName::Bot,
            IconName::TriangleAlert,
            IconName::CircleCheck,
            IconName::CircleDot,
            IconName::Circle,
            IconName::Wrench,
            IconName::LoaderCircle,
            IconName::ShieldAlert,
            IconName::Terminal,
            IconName::Key,
            IconName::X,
            IconName::ChartColumn,
            IconName::Download,
            IconName::Copy,
            IconName::Search,
            IconName::Rows3,
            IconName::Braces,
            IconName::Table,
            IconName::Database,
            IconName::ListIndentIncrease,
            IconName::ChartLine,
            IconName::ChartArea,
        ];
        for icon in named {
            let path = icon.path();
            assert!(
                super::Assets.load(&path).is_ok_and(|bytes| bytes.is_some()),
                "{path} is named but does not load"
            );
        }
    }
}
