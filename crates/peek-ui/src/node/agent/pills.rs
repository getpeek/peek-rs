//! The two pills in the node's header.
//!
//! Both hide when they would offer no choice: the provider pill below two backends, the mode
//! pill with no modes. A control that can only be set to what it already says is noise.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, SharedString, div, rems};
use peek_document::AgentProvider;
use peek_theme::ActivePeekTheme;

use super::AgentView;
use super::acp;
use super::backend::Agents;
use super::view::Backend;

/// The mode the ACP agent is in, and a menu to change it. `shift-tab` cycles the same list.
pub(super) fn mode(view: &Entity<AgentView>, cx: &mut App) -> Option<AnyElement> {
    let Backend::Acp(state) = view.read(cx).backend() else {
        return None;
    };
    let modes = state.modes().to_vec();
    if modes.is_empty() {
        return None;
    }
    let current = state.current_mode().map(str::to_string);
    let label = modes
        .iter()
        .find(|(id, _)| Some(id) == current.as_ref())
        .or_else(|| modes.first())
        .map(|(_, name)| name.clone())
        .unwrap_or_default();

    let view = view.clone();
    Some(
        pill("agent-mode", label, cx)
            .on_click(move |_, _, cx| {
                // Without a menu surface on the canvas, the pill advances through the list —
                // the same order `shift-tab` takes, so the two never disagree.
                view.update(cx, acp::cycle_mode);
            })
            .into_any_element(),
    )
}

/// Which backend this node runs on. Hidden unless there is a genuine choice.
pub(super) fn provider(view: &Entity<AgentView>, cx: &mut App) -> Option<AnyElement> {
    let available = Agents::available(cx);
    if available.len() < 2 {
        return None;
    }
    let current = match view.read(cx).backend() {
        Backend::Acp(_) => AgentProvider::Acp,
        Backend::Ollama => AgentProvider::Ollama,
        Backend::Unconfigured => return None,
    };
    Some(
        pill("agent-provider", label_of(current).to_string(), cx)
            .tooltip("The backend this node talks to")
            .into_any_element(),
    )
}

fn label_of(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Acp => "ACP",
        AgentProvider::Ollama => "Ollama",
    }
}

/// `.acp-mode-pill`: a bordered capsule on the inset fill, so the two header controls read as
/// settings rather than as more of the title.
fn pill(id: &'static str, label: String, cx: &App) -> Button {
    let theme = cx.peek_theme();
    Button::new(SharedString::from(id))
        .ghost()
        .xsmall()
        .p_0()
        .child(
            div()
                .h_flex()
                .items_center()
                .gap(rems(0.25))
                .pl(rems(0.5))
                .pr(rems(0.375))
                .py(rems(0.1875))
                .rounded_full()
                .border_1()
                .border_color(theme.node_border)
                .bg(theme.node_inset)
                .child(
                    div()
                        .text_size(rems(0.65625))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme.fg_muted)
                        .child(label),
                )
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(rems(0.75))
                        .text_color(theme.fg_subtle),
                ),
        )
}
