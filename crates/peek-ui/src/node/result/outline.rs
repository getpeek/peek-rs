//! The border traced around a selection, and the dashed preview of the one a click would make.
//!
//! `Result.css`, "Selection outline" and "Ghost outline": rather than one rectangle drawn over
//! the range, each cell on the range's boundary draws the single border side it sits on. An
//! interior cell draws nothing, so what is left is the perimeter — and a run of selected rows
//! comes out as one rounded band while two disjoint rows come out as two, with no code anywhere
//! that knows what a "band" is.
//!
//! It is an absolutely positioned overlay rather than a border on the cell itself, so turning a
//! cell's border on never moves the text inside it. gpui-component paints its own cell selection
//! the same way (`table/state.rs`).

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, div};
use peek_theme::ActivePeekTheme;

use super::selection::Edges;

/// Which of the two outlines a cell is drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outline {
    /// The live selection: solid, in the accent.
    Selected,
    /// What a press here would select: dashed, in the accent's line tint, and painted first so a
    /// solid edge wins wherever the two overlap.
    Ghost,
}

/// The overlay for one cell, or nothing when the cell is not on a boundary.
pub(super) fn overlay(kind: Outline, edges: Edges, cx: &App) -> Option<AnyElement> {
    if !edges.any() {
        return None;
    }
    let theme = cx.peek_theme();
    let color = match kind {
        Outline::Selected => theme.accent,
        Outline::Ghost => theme.accent_line,
    };
    let radius = theme.radius_selection;

    let outline = div()
        .absolute()
        .inset_0()
        .border_color(color)
        // Nothing here is `occlude`d and it carries no id, so the press still reaches the cell
        // underneath: the outline is paint, not a surface.
        .when(kind == Outline::Ghost, Styled::border_dashed)
        .when(edges.top, Styled::border_t_1)
        .when(edges.bottom, Styled::border_b_1)
        .when(edges.left, Styled::border_l_1)
        .when(edges.right, Styled::border_r_1)
        // Only the corners the selection actually turns at are rounded; a cell that is merely on
        // one side keeps its square corners so the edge runs straight into its neighbour.
        .when(edges.top && edges.left, |corner| corner.rounded_tl(radius))
        .when(edges.top && edges.right, |corner| corner.rounded_tr(radius))
        .when(edges.bottom && edges.left, |corner| {
            corner.rounded_bl(radius)
        })
        .when(edges.bottom && edges.right, |corner| {
            corner.rounded_br(radius)
        });
    Some(outline.into_any_element())
}
