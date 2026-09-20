//! Grouping, ungrouping, the regions picker and the regions toggle.
//!
//! These act on the selection and the settings, so they live on the canvas like the rest of
//! `dispatch/`. The decision each of them makes is in `peek_canvas::regions` — this file is
//! only the wiring plus the one thing the reference does that the model cannot: the naming
//! hand-off, where a new region opens the picker in rename mode instead of keeping the
//! placeholder name it was created with.

use gpui_kit::{Context, InteractiveElement, Task, Window};
use peek_canvas::regions::{GroupPlan, Prompt};
use peek_canvas::{Document, NewRegion};
use peek_document::RegionStatus;
use peek_ollama::Chat;

use super::CanvasView;
use crate::commands::actions;
use crate::node::agent::backend::Agents;
use crate::settings::Settings;

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(cx.listener(CanvasView::group_selection))
        .on_action(cx.listener(CanvasView::ungroup_selection))
        .on_action(cx.listener(CanvasView::open_regions_picker))
        .on_action(cx.listener(CanvasView::toggle_regions))
        .on_action(cx.listener(CanvasView::group_with_ai))
        .on_action(cx.listener(CanvasView::regroup_all_with_ai))
}

/// Which of the two AI groupings is running, so the picker can spin the button that started
/// it and refuse a second one meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grouping {
    /// Only the nodes no region holds; the existing regions stay.
    Ungrouped,
    /// The whole page, re-partitioned from scratch.
    All,
}

/// A grouping in flight. Dropping it cancels the request — which is what closing the document
/// mid-ask does, rather than leaving the model working on a page nobody is looking at.
pub(crate) struct GroupingRun {
    kind: Grouping,
    _task: Task<()>,
}

impl std::fmt::Debug for GroupingRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GroupingRun")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl CanvasView {
    /// ⌘G. A selection touching exactly one region grows it — that is how a region is extended
    /// by hand — and anything else mints a new one and asks what to call it.
    fn group_selection(
        &mut self,
        _: &actions::region::GroupSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Settings::get(cx).canvas.enable_regions {
            return;
        }
        let members: Vec<_> = self.document.read(cx).selected().iter().cloned().collect();
        match self.document.read(cx).group_plan() {
            GroupPlan::Unavailable => (),
            GroupPlan::FoldInto(region) => {
                self.document.update(cx, |document, cx| {
                    document.add_to_region(&region, members);
                    document.checkpoint();
                    cx.notify();
                });
                // Folding keeps the target's name, so there is no rename hand-off to say what
                // absorbed the nodes. The ring is the only cue.
                self.flash_region(region, cx);
            }
            GroupPlan::Create => {
                let name = self.next_region_name(cx);
                let region = self.document.update(cx, |document, cx| {
                    let region = document.group_nodes(
                        members,
                        NewRegion {
                            name: name.clone(),
                            desc: String::new(),
                            // Made by hand, so it needs no review — only a name.
                            status: RegionStatus::Confirmed,
                        },
                    );
                    document.checkpoint();
                    cx.notify();
                    region
                });
                self.regions.update(cx, |panel, cx| {
                    panel.open_renaming(region, &name, window, cx);
                });
            }
        }
    }

    /// `Region {n}`, as `useGroupSelection.ts` names one. A placeholder that lasts exactly as
    /// long as it takes to type over it in the picker this opens.
    fn next_region_name(&self, cx: &Context<Self>) -> String {
        format!("Region {}", self.document.read(cx).regions().len() + 1)
    }

    /// ⌘⇧G. Pulls the selected nodes out of whatever regions hold them, dropping the ones that
    /// empties — the reference's `removeFromRegions`.
    fn ungroup_selection(
        &mut self,
        _: &actions::region::UngroupSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Settings::get(cx).canvas.enable_regions {
            return;
        }
        let members = self.document.read(cx).grouped_selection();
        self.document.update(cx, |document: &mut Document, cx| {
            if document.remove_from_regions(&members) {
                document.checkpoint();
                cx.notify();
            }
        });
    }

    /// Asks the local model to slot the ungrouped nodes into regions — `useGroupWithAi.ts`.
    fn group_with_ai(
        &mut self,
        _: &actions::region::GroupWithAi,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_grouping(Grouping::Ungrouped, cx);
    }

    /// Asks it to re-partition the whole page — `useRegroupAllWithAi.ts`.
    fn regroup_all_with_ai(
        &mut self,
        _: &actions::region::RegroupAllWithAi,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_grouping(Grouping::All, cx);
    }

    /// One ask, and what to do with the answer.
    ///
    /// The model decides the grouping, not only the names; a model that cannot be reached or
    /// answers with nothing usable is not an error, because the geometric clustering behind it
    /// produces the same kind of suggestion. Either way the regions land as `Suggested`, which
    /// is what the Keep / Rename / Dismiss card over each one exists for.
    fn start_grouping(&mut self, kind: Grouping, cx: &mut Context<Self>) {
        if !Settings::get(cx).canvas.enable_regions || self.grouping.is_some() {
            return;
        }
        let Some(session) = Agents::ollama(cx) else {
            log::info!("peek: no local model is configured, so there is nothing to group with");
            return;
        };
        let prompt = match kind {
            Grouping::Ungrouped => Prompt::extend(self.document.read(cx)),
            Grouping::All => Prompt::partition(self.document.read(cx)),
        };
        let Some(prompt) = prompt else {
            return;
        };

        let task = cx.spawn(async move |this, cx| {
            let answer = session
                .ask(Chat::new().system(prompt.system()).user(prompt.user()))
                .await;
            if let Err(error) = &answer {
                log::info!(
                    "peek: the model could not group the page ({error}); clustering instead"
                );
            }
            let reply = answer.unwrap_or_default();

            this.update(cx, |this, cx| {
                this.grouping = None;
                this.document.update(cx, |document, cx| {
                    let plan = prompt
                        .parse(&reply)
                        .unwrap_or_else(|| prompt.fallback(document));
                    if document.apply_grouping(plan) {
                        document.checkpoint();
                        cx.notify();
                    }
                });
                cx.notify();
            })
            .ok();
        });
        self.grouping = Some(GroupingRun { kind, _task: task });
        cx.notify();
    }

    /// The grouping the picker should show as running, if any.
    pub(crate) fn grouping(&self) -> Option<Grouping> {
        self.grouping.as_ref().map(|run| run.kind)
    }

    /// Takes the picker down if it is up, reporting whether there was one. Escape and the
    /// regions toggle both need to.
    pub(crate) fn close_regions_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.regions.update(cx, |panel, cx| {
            let was_open = panel.is_open();
            panel.close(window, cx);
            was_open
        })
    }

    fn open_regions_picker(
        &mut self,
        _: &actions::region::OpenPicker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Settings::get(cx).canvas.enable_regions {
            return;
        }
        self.regions
            .update(cx, |panel, cx| panel.toggle(window, cx));
    }

    /// Switches the whole feature off, as the reference's palette command does. Every surface
    /// reads the setting, so nothing else has to be torn down — except the picker, which would
    /// otherwise be left up with nothing rendering its trigger.
    fn toggle_regions(
        &mut self,
        _: &actions::settings::ToggleRegions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !Settings::get(cx).canvas.enable_regions;
        if let Err(error) = Settings::update(cx, |config| config.canvas.enable_regions = next) {
            // Read-only is the expected answer for most of this build's life, so this is not a
            // warning; the change still holds for the session either way.
            log::debug!("peek: regions preference not saved: {error}");
        }
        if !next {
            self.close_regions_picker(window, cx);
        }
        cx.notify();
    }
}
