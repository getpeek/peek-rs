//! Peek's themes: six built-in [`ThemeSpec`] tables, the runtime [`PeekTheme`] the canvas reads
//! through [`ActivePeekTheme`], the projection onto gpui-component's `ThemeConfig` so every
//! widget matches, and the [`ThemeService`] that previews, commits and applies a theme.

pub mod builtin;
mod component_map;
mod resolved;
mod service;
mod spec;

pub use component_map::to_component_config;
pub use resolved::{ActivePeekTheme, EdgeState, PeekTheme, ResolvedFrame};
pub use service::ThemeService;
pub use spec::{
    BLACK, CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator, WHITE,
};
