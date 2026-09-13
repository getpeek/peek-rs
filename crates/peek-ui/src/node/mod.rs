//! The chrome every node shares: header with a kind indicator and title, and a body slot.
//! Sizes are rem-based so the canvas' rem scope scales them with the zoom; borders stay one
//! physical pixel by design. Colours come from the Peek theme, never from literals.

pub(crate) mod agent;
pub(crate) mod barchart;
pub(crate) mod draw;
pub(crate) mod kind;
pub(crate) mod placeholder;
pub(crate) mod query;
pub(crate) mod query_error;
pub(crate) mod result;
pub(crate) mod state;
pub(crate) mod table_definition;
pub(crate) mod text;
pub(crate) mod variable;

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, Hsla, Pixels, Rems, SharedString, Window, div, rems};
use peek_document::{Node, NodeType};
use peek_theme::{ActivePeekTheme, PeekTheme, ResolvedFrame, TypeIndicator};

/// Rem size the node chrome was designed against; radii in the theme are pixels at zoom 1.
const BASE_REM: f32 = 16.0;

/// World units a kind must keep clear of its bottom edge if it puts controls there.
/// `peek_canvas::hit` claims that band for resizing, so a press inside it never reaches the
/// element — matching `RESIZE_EDGE_SCREEN_INSET` at zoom 1, which is where a node is framed
/// when it is placed. Zoomed further out the band covers more world units than this and eats
/// into a footer; precise clicking at that zoom is not the case worth optimising for.
const RESIZE_FOOTER_CLEARANCE: f32 = 12.0;

/// A themed pixel length as rems, so it tracks the canvas' rem scope instead of staying the
/// same number of screen pixels as the camera zooms out.
pub(crate) fn scaled(length: Pixels) -> Rems {
    rems(f32::from(length) / BASE_REM)
}

/// A length designed in pixels, as the camera should draw it.
///
/// A few gpui-component APIs take screen pixels and offer no rem-relative form — a markdown
/// heading's size, a multi-line input's text inset — so inside the canvas they have to be scaled
/// by hand. Reads the rem scope the node is being laid out in, which is every `render` below
/// [`crate::canvas`]; [`kind::NodeContext::zoom`] is the answer outside it.
pub(crate) fn scaled_px(design: f32, window: &Window) -> Pixels {
    gpui_kit::px(design * f32::from(window.rem_size()) / BASE_REM)
}

/// Screen pixels a multi-line gpui-component input keeps between its frame and its first glyph:
/// `Size::Medium`'s `input_px` and `input_py`. The editor applies them inside itself, where no
/// style refinement reaches them.
const INPUT_INSET: (f32, f32) = (10.0, 8.0);

/// More of the same in front of a code editor's first column: the gutter it lays out for line
/// numbers, which stays one `LINE_NUMBER_RIGHT_MARGIN` wide even when they are switched off.
const CODE_GUTTER: f32 = 10.0;

pub(crate) trait TextInsetExt: Styled + Sized {
    /// Turns a multi-line input's fixed text inset into one the camera scales.
    fn scaled_text_inset(self, window: &Window) -> Self {
        let (x, y) = INPUT_INSET;
        self.pulled_back(x, y, window)
    }

    /// The same for a code editor, which insets its text a second time for the gutter.
    fn scaled_code_inset(self, window: &Window) -> Self {
        let (x, y) = INPUT_INSET;
        self.pulled_back(x + CODE_GUTTER, y, window)
    }

    /// Pulls the frame out by the part of a fixed inset the camera should have taken away, which
    /// leaves the text `inset * zoom` from the edge and the frame no taller than the text needs.
    /// The overhang is real, so an input styled this way must paint no background or border of
    /// its own — draw those on the element around it instead.
    fn pulled_back(self, x: f32, y: f32, window: &Window) -> Self {
        self.mx(scaled_px(x, window) - gpui_kit::px(x))
            .my(scaled_px(y, window) - gpui_kit::px(y))
    }
}

impl<T: Styled> TextInsetExt for T {}

/// The chrome around a node body. Kinds contribute a `body` element and optional header
/// controls; everything else — frame, header, brackets — is identical for all of them.
#[derive(IntoElement)]
pub(crate) struct NodeShell {
    id: SharedString,
    node_type: Option<NodeType>,
    title: SharedString,
    header_extras: Option<AnyElement>,
    body: AnyElement,
    selected: bool,
}

impl NodeShell {
    pub(crate) fn new(node: &Node, selected: bool, body: AnyElement, cx: &App) -> Self {
        Self {
            id: SharedString::from(node.id.to_string()),
            node_type: node.node_type(),
            title: kind::title(node, cx).into(),
            header_extras: None,
            body,
            selected,
        }
    }

    /// Controls the kind adds to the right of its header (a live toggle, a chart-type switch).
    #[must_use]
    pub(crate) fn header_extras(mut self, extras: Option<AnyElement>) -> Self {
        self.header_extras = extras;
        self
    }
}

fn indicator(kind: TypeIndicator, accent: Hsla) -> Div {
    match kind {
        TypeIndicator::Dot => div().size(rems(0.5)).rounded_full().bg(accent),
        TypeIndicator::Tick => div().w(rems(0.1875)).h(rems(0.875)).bg(accent),
    }
}

/// Terminal's and Blueprint's eight corner segments, drawn as absolutely placed slivers.
fn corner_brackets(theme: &PeekTheme, selected: bool) -> Vec<Div> {
    let ResolvedFrame::Brackets {
        color,
        length,
        thickness,
        selected_length,
        selected_thickness,
    } = theme.node_frame
    else {
        return Vec::new();
    };
    let (color, length, thickness) = if selected {
        (theme.selection, selected_length, selected_thickness)
    } else {
        (color, length, thickness)
    };
    let long = rems(length / BASE_REM);
    let short = rems(thickness / BASE_REM);
    let horizontal = || div().absolute().w(long).h(short).bg(color);
    let vertical = || div().absolute().w(short).h(long).bg(color);
    vec![
        horizontal().top_0().left_0(),
        vertical().top_0().left_0(),
        horizontal().top_0().right_0(),
        vertical().top_0().right_0(),
        horizontal().bottom_0().left_0(),
        vertical().bottom_0().left_0(),
        horizontal().bottom_0().right_0(),
        vertical().bottom_0().right_0(),
    ]
}

impl RenderOnce for NodeShell {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.peek_theme();
        let accent = theme.node_type(self.node_type);
        let label = self.node_type.map_or("NODE", NodeType::label);
        let radius = scaled(theme.radius_node);
        let border = if self.selected {
            theme.node_border_strong
        } else {
            theme.node_border
        };
        let hover_border = theme.node_border_strong;

        div()
            .id(self.id)
            .test_support()
            .v_flex()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(radius)
            .bg(theme.node_bg)
            .border_1()
            .border_color(border)
            .text_size(rems(0.8125))
            .line_height(rems(1.25))
            .text_color(theme.fg)
            .hover(move |style| style.border_color(hover_border))
            .child(
                div()
                    .h_flex()
                    .gap(rems(0.5))
                    .px(rems(0.75))
                    .h(rems(2.0))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(theme.node_border)
                    .child(indicator(theme.type_indicator, accent))
                    .child(
                        div()
                            .text_size(rems(0.625))
                            .text_color(theme.fg_muted)
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(self.title))
                    .children(self.header_extras),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(self.body))
            .children(corner_brackets(theme, self.selected))
    }
}
