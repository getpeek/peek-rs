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
