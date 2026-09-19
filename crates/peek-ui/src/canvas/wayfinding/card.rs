//! The review card over a suggested region — the port of `RegionHalos.tsx`'s `SuggestedRegion`
//! and `.wf-suggest-card`.
//!
//! `RegionStatus::Suggested` is not an AI-only state here: `tools/regions.rs` defaults
//! `suggested` to true, so every region an agent groups through MCP arrives as a proposal. This
//! is where it is accepted, named or dismissed — without it, a grouped page fills up with
//! dashed boxes nothing can resolve.
//!
//! Chrome rather than content, by the rule in `docs/canvas.md`: it is a surface you *act
//! through*, so it lays out at rem 1 outside the camera's scope and keeps a constant screen
//! size. The reference has to counter-scale by `1 / zoom` to get the same thing, because its
//! card lives inside the zoomed viewport.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, BoxShadow, Context, Entity, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent,
    Pixels, SharedString, Subscription, WeakEntity, Window, div, point, px,
};
use peek_canvas::regions::derive::Derived;
use peek_document::{RegionId, RegionStatus};
use peek_theme::ActivePeekTheme;

use crate::canvas::{CanvasView, screen_rect};

/// `.wf-suggest-card`'s width, and how far below the region's top edge it sits.
const WIDTH: Pixels = px(280.0);
const TOP_OFFSET: Pixels = px(16.0);

pub(crate) struct SuggestionCards {
    canvas: WeakEntity<CanvasView>,
    /// At most one card can be renaming, so one field serves all of them.
    renaming: Option<RegionId>,
    rename: Entity<InputState>,
    _rename: Subscription,
}

impl std::fmt::Debug for SuggestionCards {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SuggestionCards")
            .field("renaming", &self.renaming)
            .finish_non_exhaustive()
    }
}

impl SuggestionCards {
    pub(crate) fn new(
        canvas: WeakEntity<CanvasView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rename = cx.new(|cx| InputState::new(window, cx));
        let typing = cx.subscribe_in(&rename, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_rename(window, cx);
            }
        });
        Self {
            canvas,
            renaming: None,
            rename,
            _rename: typing,
        }
    }

    fn start_rename(
        &mut self,
        region: RegionId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.renaming = Some(region);
        let rename = self.rename.clone();
        let name = name.to_string();
        rename.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Renaming *is* accepting, which is why there is no Keep step after it: typing a name over
    /// a proposal is how the reference confirms one (`useRegionActions.ts`).
    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(region) = self.renaming.take() else {
            return;
        };
        let name = self.rename.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.confirm(&region, cx);
        } else {
            self.edit(cx, |document, cx| {
                if document.rename_region(&region, name) {
                    document.checkpoint();
                    cx.notify();
                }
            });
        }
        self.hand_focus_back(window, cx);
        cx.notify();
    }

    fn confirm(&mut self, region: &RegionId, cx: &mut Context<Self>) {
        self.edit(cx, |document, cx| {
            if document.confirm_region(region) {
                document.checkpoint();
                cx.notify();
            }
        });
        cx.notify();
    }

    fn dismiss(&mut self, region: &RegionId, cx: &mut Context<Self>) {
        self.edit(cx, |document, cx| {
            if document.remove_region(region) {
                document.checkpoint();
                cx.notify();
            }
        });
        cx.notify();
    }

    fn edit(
        &self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut peek_canvas::Document, &mut Context<peek_canvas::Document>),
    ) {
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        let document = canvas.read(cx).document().clone();
        document.update(cx, work);
    }

    fn hand_focus_back(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.update(cx, |canvas, cx| canvas.take_focus(window, cx));
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" || self.renaming.take().is_none() {
            return;
        }
        self.hand_focus_back(window, cx);
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for SuggestionCards {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layer = div().id("suggestion-cards").absolute().inset_0();
        let Some(canvas) = self.canvas.upgrade() else {
            return layer.invisible();
        };
        let (camera, regions, nodes) = {
            let canvas = canvas.read(cx);
            (
                canvas.camera(),
                canvas.derived_regions(cx),
                canvas.document().read(cx).nodes().to_vec(),
            )
        };
        let cards: Vec<_> = regions
            .iter()
            .filter(|region| region.status == RegionStatus::Suggested)
            .map(|region| self.card(region, &nodes, camera, cx))
            .collect();
        if cards.is_empty() {
            return layer.invisible();
        }
        layer
            .children(cards)
            .on_key_down(cx.listener(Self::on_key_down))
    }
}

impl SuggestionCards {
    fn card(
        &self,
        region: &Derived,
        nodes: &[peek_document::Node],
        camera: peek_canvas::Camera,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.peek_theme().clone();
        let box_ = screen_rect(camera, region.bbox(nodes));
        let renaming = self.renaming.as_ref() == Some(&region.id);
        let id = region.id.clone();
        let for_rename = (region.id.clone(), region.name.clone());
        let for_dismiss = region.id.clone();

        div()
            .id(SharedString::from(format!("suggestion-{}", region.id)))
            .test_support()
            .absolute()
            .left(box_.origin.x + box_.size.width / 2.0 - WIDTH / 2.0)
            .top(box_.origin.y + TOP_OFFSET)
            .w(WIDTH)
            .occlude()
            .v_flex()
            .gap(px(9.0))
            .p(px(12.0))
            .bg(theme.node_bg)
            .border_1()
            .border_color(theme.accent_line)
            .rounded(theme.radius_card)
            .when_some(theme.node_shadow, |this, (offset_y, blur, color)| {
                this.shadow(vec![BoxShadow {
                    color,
                    offset: point(px(0.0), offset_y),
                    blur_radius: blur,
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            // The card floats over the canvas, so without this a press on it starts a marquee
            // underneath and clears the selection the suggestion is about.
            .on_mouse_down(
                MouseButton::Left,
                |_: &MouseDownEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                    cx.stop_propagation();
                },
            )
            .child(
                div()
                    .h_flex()
                    .gap(px(7.0))
                    .text_size(px(10.5))
                    .font_family("Monaspace Krypton")
                    .text_color(theme.accent)
                    .child(Icon::new(IconName::Sparkles).size(px(12.0)))
                    .child(format!(
                        "AI suggests grouping these {} nodes",
                        region.member_ids.len()
                    )),
            )
            .child(if renaming {
                div()
                    .id("suggestion-rename")
                    .test_support()
                    .child(Input::new(&self.rename).appearance(false))
                    .into_any_element()
            } else {
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg)
                    .child(format!("\u{201c}{}\u{201d}", region.name))
                    .into_any_element()
            })
            .when(!renaming, |this| {
                this.child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .child(
                            action_button("suggestion-keep", "\u{2713} Keep")
                                .bg(theme.accent_bg)
                                .border_color(theme.accent_line)
                                .text_color(theme.fg)
                                .on_click(cx.listener(move |this, _, _, cx| this.confirm(&id, cx))),
                        )
                        .child(
                            action_button("suggestion-rename-button", "Rename")
                                .bg(theme.node_bg_2)
                                .border_color(theme.node_border_strong)
                                .text_color(theme.fg)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let (id, name) = &for_rename;
                                    this.start_rename(id.clone(), name, window, cx);
                                })),
                        )
                        .child(
                            action_button("suggestion-dismiss", "Dismiss")
                                .text_color(theme.fg_muted)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dismiss(&for_dismiss, cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

/// `.sc-actions button`: a bordered pill, or a bare one for the ghost action.
fn action_button(
    id: &'static str,
    label: &'static str,
) -> impl IntoElement + Styled + StatefulInteractiveElement {
    div()
        .id(id)
        .test_support()
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .cursor_pointer()
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .child(label)
}
