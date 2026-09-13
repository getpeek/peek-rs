//! The pointer/wheel state machine, kept pure so React Flow's rules (`ReactFlowCanvas.tsx`,
//! `useRubberBandSelect.ts`) are unit-tested here and the gpui layer only translates events
//! in and applies [`Effect`]s out.

use std::collections::BTreeSet;

use peek_document::geometry::{Point, Rect, Size};
use peek_document::{EdgeId, NodeId, NodeType};

use crate::camera::Camera;
use crate::hit::{Corner, Hit, NodeHit, NodeRegion};

/// Movement below this is a click, not a drag (`DRAG_THRESHOLD_PX`).
pub const DRAG_THRESHOLD: f64 = 4.0;
/// Pixels per `ScrollDelta::Lines` unit for mouse wheels (React Flow's `deltaMode` normaliser).
pub const LINE_HEIGHT: f64 = 20.0;
/// cmd/ctrl + wheel: zoom factor per pixel of vertical delta, `2^(-dy * k)`.
pub const WHEEL_ZOOM_EXPONENT: f64 = 0.01;
/// Largest zoom change a single event may apply.
pub const MAX_ZOOM_STEP: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    /// cmd on macOS, ctrl elsewhere.
    pub secondary: bool,
    pub control: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Down {
        button: Button,
        screen: Point,
        modifiers: Modifiers,
        /// What the pointer is over: a node region, an edge, or bare canvas.
        on: Option<Hit>,
    },
    Move {
        screen: Point,
    },
    Up {
        button: Button,
        screen: Point,
    },
    Wheel {
        screen: Point,
        /// Already in pixels (`ScrollDelta::pixel_delta` with [`LINE_HEIGHT`] for lines).
        delta: Point,
        modifiers: Modifiers,
        phase: Phase,
    },
    Pinch {
        screen: Point,
        /// `PinchEvent::delta`: 0.1 means 10 % larger.
        delta: f64,
        phase: Phase,
    },
    /// Arms or cancels place mode (`usePlaceTool`): the next press places a node.
    ArmTool(Option<NodeType>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GestureConfig {
    pub camera_locked: bool,
    pub space_held: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Interaction {
    #[default]
    Idle,
    /// Button down, not yet past the drag threshold.
    PendingPress {
        button: Button,
        origin: Point,
        modifiers: Modifiers,
        on: Option<Hit>,
    },
    Panning {
        last: Point,
    },
    Marquee {
        origin: Point,
        current: Point,
        /// Selection to extend from when shift was held (`useRubberBandSelect`).
        baseline: BTreeSet<NodeId>,
    },
    DraggingNodes {
        last: Point,
        ids: Vec<NodeId>,
    },
    ResizingNode {
        id: NodeId,
        corner: Corner,
        /// Press position and the node's bounds at that moment, both in world units, so a
        /// resize is always computed from the start rather than accumulated per frame.
        origin: Point,
        start: Rect,
    },
    /// The draw tool is armed. `samples` is empty until a press lands and is emptied again by
    /// the commit, without leaving the state: `useDrawTool` never clears place mode, so draw is
    /// the one sticky tool.
    Drawing {
        /// World units, converted on arrival. The reference buffers client coordinates and
        /// converts the batch at commit, which is only equivalent because its camera cannot
        /// move mid-stroke; converting here keeps the camera off the commit path so a pan
        /// during a stroke cannot shear it.
        samples: Vec<Point>,
    },
    /// A tool is armed; `origin` is set once the press lands.
    Placing {
        node_type: NodeType,
        origin: Option<Point>,
        /// Whether the drag has passed the threshold and the node it is sizing exists. The id
        /// stays with the view that minted it: the reducer only needs to know that the next
        /// move resizes rather than creates.
        placed: bool,
    },
}

impl Interaction {
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// The marquee rectangle in screen space while one is being drawn.
    #[must_use]
    pub fn marquee_screen_rect(&self) -> Option<Rect> {
        match self {
            Self::Marquee {
                origin, current, ..
            } => Some(Rect::from_corners(*origin, *current)),
            _ => None,
        }
    }

    #[must_use]
    pub fn armed_tool(&self) -> Option<NodeType> {
        match self {
            Self::Placing { node_type, .. } => Some(*node_type),
            Self::Drawing { .. } => Some(NodeType::Draw),
            _ => None,
        }
    }

    /// The stroke being drawn, in world units, for the live preview (`LiveStroke.tsx`).
    #[must_use]
    pub fn stroke_world(&self) -> &[Point] {
        match self {
            Self::Drawing { samples } => samples,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Pan(Point),
    ZoomAbout {
        anchor: Point,
        factor: f64,
    },
    /// World-space marquee plus the selection to extend from.
    MarqueeChanged {
        world: Rect,
        baseline: BTreeSet<NodeId>,
    },
    /// World-space delta for the given nodes.
    TranslateNodes {
        ids: Vec<NodeId>,
        delta: Point,
    },
    /// New world-space bounds for one node, already reflecting the grabbed corner.
    ResizeNode {
        id: NodeId,
        bounds: Rect,
    },
    /// Create a node of this kind with these world bounds and select it: the first frame of a
    /// placement drag, or the whole of a placement click.
    PlaceNode {
        node_type: NodeType,
        world: Rect,
    },
    /// New world bounds for the node the running placement created, once per move.
    ResizePlacement {
        world: Rect,
    },
    /// The placement is over: seal its undo step, frame the node and hand it focus.
    CommitPlacement,
    /// Commit one freehand stroke: at least two world-space samples, in the order drawn.
    PlaceDrawing {
        points: Vec<Point>,
    },
    SelectOnly(Vec<NodeId>),
    ExtendSelection(Vec<NodeId>),
    ToggleSelect(NodeId),
    /// Selects one edge and clears the node selection (React Flow's `addSelectedEdges`).
    SelectEdgeOnly(EdgeId),
    ToggleEdgeSelect(EdgeId),
    DeselectAll,
    /// The viewport may have changed and should be committed to the document.
    GestureEnded,
}

/// Advances the interaction with one input. `selected` is the current selection (needed to
/// decide what a drag moves and what shift-marquee extends).
pub fn reduce(
    state: &mut Interaction,
    config: &GestureConfig,
    camera: Camera,
    selected: &BTreeSet<NodeId>,
    input: Input,
) -> Vec<Effect> {
    if let Interaction::Drawing { samples } = state
        && let Some(effects) = on_drawing(samples, camera, &input)
    {
        return effects;
    }
    match input {
        Input::Down {
            button,
            screen,
            modifiers,
            on,
        } => on_down(state, config, button, screen, modifiers, on),
        Input::Move { screen } => on_move(state, camera, selected, screen),
        Input::Up { button, screen } => on_up(state, camera, button, screen),
        Input::Wheel {
            screen,
            delta,
            modifiers,
            phase,
        } => on_wheel(config, screen, delta, modifiers, phase),
        Input::Pinch {
            screen,
            delta,
            phase,
        } => on_pinch(config, screen, delta, phase),
        Input::ArmTool(node_type) => {
            *state = match node_type {
                Some(NodeType::Draw) => Interaction::Drawing {
                    samples: Vec::new(),
                },
                Some(node_type) => Interaction::Placing {
                    node_type,
                    origin: None,
                    placed: false,
                },
                None => Interaction::Idle,
            };
            Vec::new()
        }
    }
}

/// The draw tool owns the pointer while it is armed, which is what `useDrawTool`'s
/// `stopPropagation` buys: no press reaches the pan, marquee, drag or resize paths, so a stroke
/// started over a node draws rather than moving it. `None` hands the input back to [`reduce`],
/// which is how wheel and pinch keep panning and zooming while the pen is armed.
///
/// **A non-left press does nothing**, a deliberate delta. The reference returns before its
/// `stopPropagation`, so React Flow's `panOnDrag={[1, 2]}` still pans mid-mode; matching that
/// would mean teaching [`Interaction::Panning`] to return here on release. Two-finger scroll and
/// pinch are unaffected either way, so the loss is middle-drag, right-drag and space-drag until
/// escape.
fn on_drawing(samples: &mut Vec<Point>, camera: Camera, input: &Input) -> Option<Vec<Effect>> {
    match input {
        Input::Down {
            button: Button::Left,
            screen,
            ..
        } => {
            samples.clear();
            samples.push(camera.screen_to_world(*screen));
            Some(Vec::new())
        }
        // Every move is a sample: the reference applies no drag threshold, and a stroke's shape
        // is exactly the samples it was given. A sample identical to the one before it is
        // dropped, as `getStrokePoints` drops it anyway — which is also what keeps a stationary
        // click from committing a degenerate one-point dot.
        Input::Move { screen } if !samples.is_empty() => {
            let world = camera.screen_to_world(*screen);
            if samples.last() != Some(&world) {
                samples.push(world);
            }
            Some(Vec::new())
        }
        Input::Up {
            button: Button::Left,
            ..
        } => {
            let points = std::mem::take(samples);
            if points.len() < 2 {
                // A click draws nothing, and the tool stays armed for the next stroke.
                return Some(Vec::new());
            }
            Some(vec![Effect::PlaceDrawing { points }])
        }
        Input::Down { .. } | Input::Move { .. } | Input::Up { .. } => Some(Vec::new()),
        Input::Wheel { .. } | Input::Pinch { .. } | Input::ArmTool(_) => None,
    }
}

fn on_down(
    state: &mut Interaction,
    config: &GestureConfig,
    button: Button,
    screen: Point,
    modifiers: Modifiers,
    on: Option<Hit>,
) -> Vec<Effect> {
    if let Interaction::Placing { origin, placed, .. } = state {
        if button == Button::Left {
            *origin = Some(screen);
            *placed = false;
        }
        return Vec::new();
    }
    if !state.is_idle() {
        return Vec::new();
    }
    let wants_pan = button != Button::Left || config.space_held;
    if wants_pan && config.camera_locked {
        return Vec::new();
    }
    if button == Button::Left && config.space_held {
        *state = Interaction::Panning { last: screen };
        return Vec::new();
    }
    *state = Interaction::PendingPress {
        button,
        origin: screen,
        modifiers,
        on,
    };
    Vec::new()
}

fn on_move(
    state: &mut Interaction,
    camera: Camera,
    selected: &BTreeSet<NodeId>,
    screen: Point,
) -> Vec<Effect> {
    match state {
        // A move before an armed tool's press has landed does nothing, and `Drawing` is
        // unreachable: [`reduce`] offers every pointer input to [`on_drawing`] first.
        Interaction::Idle
        | Interaction::Drawing { .. }
        | Interaction::Placing { origin: None, .. } => Vec::new(),
        Interaction::PendingPress {
            button,
            origin,
            modifiers,
            on,
        } => {
            if origin.distance_to(screen) < DRAG_THRESHOLD {
                return Vec::new();
            }
            let (button, origin, modifiers, on) = (*button, *origin, *modifiers, on.clone());
            begin_drag(
                state,
                camera,
                selected,
                DragStart {
                    button,
                    origin,
                    modifiers,
                    on,
                    screen,
                },
            )
        }
        Interaction::Panning { last } => {
            let delta = screen - *last;
            *last = screen;
            vec![Effect::Pan(delta)]
        }
        Interaction::Marquee {
            origin,
            current,
            baseline,
        } => {
            *current = screen;
            let world = Rect::from_corners(
                camera.screen_to_world(*origin),
                camera.screen_to_world(screen),
            );
            vec![Effect::MarqueeChanged {
                world,
                baseline: baseline.clone(),
            }]
        }
        Interaction::DraggingNodes { last, ids } => {
            let delta = (screen - *last).scaled(1.0 / camera.zoom);
            *last = screen;
            vec![Effect::TranslateNodes {
                ids: ids.clone(),
                delta,
            }]
        }
        Interaction::ResizingNode {
            id,
            corner,
            origin,
            start,
        } => {
            let delta = camera.screen_to_world(screen) - *origin;
            vec![Effect::ResizeNode {
                id: id.clone(),
                bounds: corner.resize(*start, delta),
            }]
        }
        // The node is real from the first frame past the threshold, so the drag sizes the node
        // itself rather than a preview rectangle (`usePlaceTool.ts`).
        Interaction::Placing {
            node_type,
            origin: Some(origin),
            placed,
        } => {
            let world = dragged_world(*node_type, *origin, screen, camera);
            if *placed {
                return vec![Effect::ResizePlacement { world }];
            }
            if origin.distance_to(screen) < DRAG_THRESHOLD {
                return Vec::new();
            }
            *placed = true;
            vec![Effect::PlaceNode {
                node_type: *node_type,
                world,
            }]
        }
    }
}

struct DragStart {
    button: Button,
    origin: Point,
    modifiers: Modifiers,
    on: Option<Hit>,
    screen: Point,
}

fn begin_drag(
    state: &mut Interaction,
    camera: Camera,
    selected: &BTreeSet<NodeId>,
    start: DragStart,
) -> Vec<Effect> {
    if start.button != Button::Left {
        *state = Interaction::Panning { last: start.screen };
        return vec![Effect::Pan(start.screen - start.origin)];
    }
    // A press on an edge never drags and never marquees: `useRubberBandSelect` bails on an edge
    // press and React Flow has no edge drag. It stays in `PendingPress`, so the release still
    // selects the edge.
    if matches!(start.on, Some(Hit::Edge(_))) {
        return Vec::new();
    }
    // A press on a node's body normally does the same, because the body's own content owns the
    // pointer from here. Holding the secondary modifier overrides that and turns the whole card
    // into a drag handle.
    let on_body = matches!(
        start.on,
        Some(Hit::Node(NodeHit {
            region: NodeRegion::Body,
            ..
        }))
    );
    if on_body && !start.modifiers.secondary {
        return Vec::new();
    }
    let Some(Hit::Node(hit)) = start.on else {
        let baseline = if start.modifiers.shift {
            selected.clone()
        } else {
            BTreeSet::new()
        };
        *state = Interaction::Marquee {
            origin: start.origin,
            current: start.screen,
            baseline: baseline.clone(),
        };
        let world = Rect::from_corners(
            camera.screen_to_world(start.origin),
            camera.screen_to_world(start.screen),
        );
        return vec![Effect::MarqueeChanged { world, baseline }];
    };
    if let NodeRegion::Resize(corner) = hit.region {
        // World units on both sides: the press anchor is converted once here and every later
        // move converts its own position, so the two are never mixed with raw screen pixels.
        let origin = camera.screen_to_world(start.origin);
        let delta = camera.screen_to_world(start.screen) - origin;
        *state = Interaction::ResizingNode {
            id: hit.id.clone(),
            corner,
            origin,
            start: hit.bounds,
        };
        return vec![Effect::ResizeNode {
            id: hit.id,
            bounds: corner.resize(hit.bounds, delta),
        }];
    }
    let node = hit.id;
    let mut effects = Vec::new();
    let ids: Vec<NodeId> = if selected.contains(&node) {
        selected.iter().cloned().collect()
    } else if start.modifiers.shift {
        effects.push(Effect::ExtendSelection(vec![node.clone()]));
        selected.iter().cloned().chain([node.clone()]).collect()
    } else {
        effects.push(Effect::SelectOnly(vec![node.clone()]));
        vec![node.clone()]
    };
    let delta = (start.screen - start.origin).scaled(1.0 / camera.zoom);
    effects.push(Effect::TranslateNodes {
        ids: ids.clone(),
        delta,
    });
    *state = Interaction::DraggingNodes {
        last: start.screen,
        ids,
    };
    effects
}

fn on_up(state: &mut Interaction, camera: Camera, button: Button, _screen: Point) -> Vec<Effect> {
    let previous = std::mem::take(state);
    match previous {
        // A placement whose press never landed cancels like an idle release, and `Drawing` is
        // unreachable: [`reduce`] offers every pointer input to [`on_drawing`] first.
        Interaction::Idle
        | Interaction::Placing { origin: None, .. }
        | Interaction::Drawing { .. } => Vec::new(),
        Interaction::PendingPress {
            button: pressed,
            modifiers,
            on,
            ..
        } => {
            if pressed != button || pressed != Button::Left {
                return Vec::new();
            }
            match (on, modifiers.shift) {
                (Some(Hit::Node(hit)), true) => vec![Effect::ToggleSelect(hit.id)],
                (Some(Hit::Node(hit)), false) => vec![Effect::SelectOnly(vec![hit.id])],
                (Some(Hit::Edge(id)), true) => vec![Effect::ToggleEdgeSelect(id)],
                (Some(Hit::Edge(id)), false) => vec![Effect::SelectEdgeOnly(id)],
                (None, true) => Vec::new(),
                (None, false) => vec![Effect::DeselectAll],
            }
        }
        // A release with a node already placed commits it whatever button arrived: leaving a
        // half-placed node behind would be worse than committing one the user was sizing.
        Interaction::Placing { placed: true, .. } => vec![Effect::CommitPlacement],
        Interaction::Placing {
            node_type,
            origin: Some(origin),
            placed: false,
        } => {
            if button != Button::Left {
                return Vec::new();
            }
            vec![
                Effect::PlaceNode {
                    node_type,
                    world: click_world(node_type, origin, camera),
                },
                Effect::CommitPlacement,
            ]
        }
        Interaction::Panning { .. }
        | Interaction::Marquee { .. }
        | Interaction::DraggingNodes { .. }
        | Interaction::ResizingNode { .. } => vec![Effect::GestureEnded],
    }
}

/// A placement click puts a default-size node centred on the cursor (`usePlaceTool.ts`).
fn click_world(node_type: NodeType, screen: Point, camera: Camera) -> Rect {
    let size = node_type.default_size();
    let centre = camera.screen_to_world(screen);
    Rect::new(
        Point::new(centre.x - size.width / 2.0, centre.y - size.height / 2.0),
        size,
    )
}

/// The rectangle a placement drag has covered so far, clamped to the kind's minimum.
fn dragged_world(node_type: NodeType, origin: Point, current: Point, camera: Camera) -> Rect {
    let minimum = node_type.min_size();
    let dragged = Rect::from_corners(
        camera.screen_to_world(origin),
        camera.screen_to_world(current),
    );
    Rect::new(
        dragged.origin,
        Size::new(
            dragged.size.width.max(minimum.width),
            dragged.size.height.max(minimum.height),
        ),
    )
}

fn on_wheel(
    config: &GestureConfig,
    screen: Point,
    delta: Point,
    modifiers: Modifiers,
    phase: Phase,
) -> Vec<Effect> {
    if config.camera_locked {
        return Vec::new();
    }
    let mut effects = Vec::new();
    if modifiers.secondary || modifiers.control {
        let factor = (2.0_f64).powf(-delta.y * WHEEL_ZOOM_EXPONENT);
        effects.push(Effect::ZoomAbout {
            anchor: screen,
            factor: clamp_zoom_step(factor),
        });
    } else if delta != Point::default() {
        // gpui's wheel delta is already the direction the content moves (natural scrolling
        // on macOS), so the canvas pans by it directly.
        effects.push(Effect::Pan(delta));
    }
    if phase == Phase::Ended {
        effects.push(Effect::GestureEnded);
    }
    effects
}

fn on_pinch(config: &GestureConfig, screen: Point, delta: f64, phase: Phase) -> Vec<Effect> {
    if config.camera_locked {
        return Vec::new();
    }
    let mut effects = Vec::new();
    if delta != 0.0 {
        effects.push(Effect::ZoomAbout {
            anchor: screen,
            factor: clamp_zoom_step(1.0 + delta),
        });
    }
    if phase == Phase::Ended {
        effects.push(Effect::GestureEnded);
    }
    effects
}

fn clamp_zoom_step(factor: f64) -> f64 {
    factor.clamp(1.0 / MAX_ZOOM_STEP, MAX_ZOOM_STEP)
}

/// Whether a child scroller sitting at `offset`, with `max` of overflow, can still move on
/// `delta`.
///
/// Scroll offsets run from `0` down to `-max`, and a positive delta scrolls back toward `0`, so
/// each direction has its own edge to check. The offset is clamped first, because a scroller
/// writes it unclamped when a wheel arrives and only pulls it back into range when the frame is
/// laid out again — reading that transient overscroll raw would report room that is not there.
///
/// The canvas asks this of the node under the pointer before claiming a wheel for a pan, so a
/// node body scrolls to its edge and the gesture then falls through to the camera
/// (`useScrollFallthrough`'s `canAbsorb`).
#[must_use]
pub fn has_room(delta: f64, offset: f64, max: f64) -> bool {
    let current = offset.clamp(-max, 0.0);
    if delta > 0.0 {
        return current < 0.0;
    }
    if delta < 0.0 {
        return current > -max;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: &str) -> NodeId {
        NodeId::from(raw)
    }

    fn drive(
        state: &mut Interaction,
        config: &GestureConfig,
        selected: &BTreeSet<NodeId>,
        inputs: Vec<Input>,
    ) -> Vec<Effect> {
        let camera = Camera {
            pan: Point::default(),
            zoom: 2.0,
        };
        inputs
            .into_iter()
            .flat_map(|input| reduce(state, config, camera, selected, input))
            .collect()
    }

    fn down(button: Button, at: Point, on_node: Option<&str>) -> Input {
        Input::Down {
            button,
            screen: at,
            modifiers: Modifiers::default(),
            on: on_node.map(|id| Hit::Node(header(id))),
        }
    }

    /// Existing tests press node headers; body and resize presses have their own tests.
    fn header(id: &str) -> NodeHit {
        NodeHit {
            id: NodeId::from(id),
            region: NodeRegion::Header,
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
        }
    }

    fn shift_down(at: Point, on_node: Option<&str>) -> Input {
        Input::Down {
            button: Button::Left,
            screen: at,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            on: on_node.map(|id| Hit::Node(header(id))),
        }
    }

    fn down_on_edge(at: Point, id: &str, shift: bool) -> Input {
        Input::Down {
            button: Button::Left,
            screen: at,
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
            on: Some(Hit::Edge(EdgeId::from(id))),
        }
    }

    #[test]
    fn clicking_an_edge_selects_only_it() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                down_on_edge(Point::new(20.0, 20.0), "a->b", false),
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(20.0, 20.0),
                },
            ],
        );

        assert_eq!(effects, vec![Effect::SelectEdgeOnly(EdgeId::from("a->b"))]);
        assert!(state.is_idle());
    }

    #[test]
    fn shift_clicking_an_edge_toggles_it() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                down_on_edge(Point::new(20.0, 20.0), "a->b", true),
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(20.0, 20.0),
                },
            ],
        );

        assert_eq!(
            effects,
            vec![Effect::ToggleEdgeSelect(EdgeId::from("a->b"))]
        );
    }

    /// The same rule a body press follows: the drag is inert and the release still selects.
    #[test]
    fn dragging_from_an_edge_never_marquees_and_still_selects_on_release() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                down_on_edge(Point::new(20.0, 20.0), "a->b", false),
                Input::Move {
                    screen: Point::new(200.0, 200.0),
                },
            ],
        );
        assert!(
            effects.is_empty(),
            "no marquee and no node drag: {effects:?}"
        );

        let release = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![Input::Up {
                button: Button::Left,
                screen: Point::new(200.0, 200.0),
            }],
        );
        assert_eq!(release, vec![Effect::SelectEdgeOnly(EdgeId::from("a->b"))]);
    }

    #[test]
    fn short_left_press_on_empty_canvas_deselects() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                down(Button::Left, Point::new(0.0, 0.0), None),
                Input::Move {
                    screen: Point::new(2.0, 2.0),
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(2.0, 2.0),
                },
            ],
        );
        assert_eq!(effects, vec![Effect::DeselectAll]);
        assert!(state.is_idle());
    }

    #[test]
    fn long_left_drag_on_empty_canvas_is_a_marquee_in_world_space() {
        let mut state = Interaction::default();
        let selected = BTreeSet::from([id("keep")]);
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                shift_down(Point::new(10.0, 10.0), None),
                Input::Move {
                    screen: Point::new(30.0, 50.0),
                },
            ],
        );
        assert_eq!(
            effects,
            vec![Effect::MarqueeChanged {
                world: Rect::from_corners(Point::new(5.0, 5.0), Point::new(15.0, 25.0)),
                baseline: selected.clone(),
            }]
        );
        assert_eq!(
            state.marquee_screen_rect(),
            Some(Rect::from_corners(
                Point::new(10.0, 10.0),
                Point::new(30.0, 50.0)
            ))
        );
        let end = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![Input::Up {
                button: Button::Left,
                screen: Point::new(30.0, 50.0),
            }],
        );
        assert_eq!(end, vec![Effect::GestureEnded]);
    }

    #[test]
    fn clicking_nodes_selects_and_shift_toggles() {
        let mut state = Interaction::default();
        let click = |on: &str, shift: bool| {
            vec![
                if shift {
                    shift_down(Point::default(), Some(on))
                } else {
                    down(Button::Left, Point::default(), Some(on))
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::default(),
                },
            ]
        };
        let plain = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            click("a", false),
        );
        assert_eq!(plain, vec![Effect::SelectOnly(vec![id("a")])]);
        let toggled = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            click("b", true),
        );
        assert_eq!(toggled, vec![Effect::ToggleSelect(id("b"))]);
    }

    #[test]
    fn dragging_a_node_moves_the_selection_in_world_units() {
        let mut state = Interaction::default();
        let selected = BTreeSet::from([id("a"), id("b")]);
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                down(Button::Left, Point::new(0.0, 0.0), Some("a")),
                Input::Move {
                    screen: Point::new(10.0, 0.0),
                },
                Input::Move {
                    screen: Point::new(14.0, 2.0),
                },
            ],
        );
        assert_eq!(
            effects,
            vec![
                Effect::TranslateNodes {
                    ids: vec![id("a"), id("b")],
                    delta: Point::new(5.0, 0.0),
                },
                Effect::TranslateNodes {
                    ids: vec![id("a"), id("b")],
                    delta: Point::new(2.0, 1.0),
                },
            ]
        );
    }

    #[test]
    fn dragging_an_unselected_node_selects_it_first() {
        let mut state = Interaction::default();
        let selected = BTreeSet::from([id("other")]);
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                down(Button::Left, Point::new(0.0, 0.0), Some("a")),
                Input::Move {
                    screen: Point::new(0.0, 8.0),
                },
            ],
        );
        assert_eq!(effects[0], Effect::SelectOnly(vec![id("a")]));
        assert!(matches!(&effects[1], Effect::TranslateNodes { ids, .. } if ids == &vec![id("a")]));
    }

    #[test]
    fn middle_drag_and_space_drag_pan_unless_locked() {
        for button in [Button::Middle, Button::Right] {
            let mut state = Interaction::default();
            let effects = drive(
                &mut state,
                &GestureConfig::default(),
                &BTreeSet::new(),
                vec![
                    down(button, Point::new(0.0, 0.0), None),
                    Input::Move {
                        screen: Point::new(10.0, 0.0),
                    },
                    Input::Move {
                        screen: Point::new(15.0, 0.0),
                    },
                ],
            );
            assert_eq!(
                effects,
                vec![
                    Effect::Pan(Point::new(10.0, 0.0)),
                    Effect::Pan(Point::new(5.0, 0.0))
                ]
            );
        }

        let mut state = Interaction::default();
        let space = GestureConfig {
            space_held: true,
            ..GestureConfig::default()
        };
        let effects = drive(
            &mut state,
            &space,
            &BTreeSet::new(),
            vec![
                down(Button::Left, Point::new(0.0, 0.0), None),
                Input::Move {
                    screen: Point::new(1.0, 1.0),
                },
            ],
        );
        assert_eq!(effects, vec![Effect::Pan(Point::new(1.0, 1.0))]);

        let mut state = Interaction::default();
        let locked = GestureConfig {
            camera_locked: true,
            ..GestureConfig::default()
        };
        let effects = drive(
            &mut state,
            &locked,
            &BTreeSet::new(),
            vec![
                down(Button::Middle, Point::new(0.0, 0.0), None),
                Input::Move {
                    screen: Point::new(50.0, 0.0),
                },
            ],
        );
        assert!(effects.is_empty());
    }

    #[test]
    fn wheel_pans_and_modified_wheel_zooms() {
        let wheel = |delta: Point, secondary: bool, phase: Phase| Input::Wheel {
            screen: Point::new(100.0, 100.0),
            delta,
            modifiers: Modifiers {
                secondary,
                ..Modifiers::default()
            },
            phase,
        };
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                wheel(Point::new(0.0, 10.0), false, Phase::Moved),
                wheel(Point::new(0.0, -100.0), true, Phase::Ended),
            ],
        );
        assert_eq!(effects[0], Effect::Pan(Point::new(0.0, 10.0)));
        assert_eq!(
            effects[1],
            Effect::ZoomAbout {
                anchor: Point::new(100.0, 100.0),
                factor: 2.0,
            }
        );
        assert_eq!(effects[2], Effect::GestureEnded);

        let locked = GestureConfig {
            camera_locked: true,
            ..GestureConfig::default()
        };
        assert!(
            drive(
                &mut state,
                &locked,
                &BTreeSet::new(),
                vec![wheel(Point::new(0.0, 10.0), false, Phase::Moved)]
            )
            .is_empty()
        );
    }

    #[test]
    fn pinch_zooms_about_the_pointer() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![Input::Pinch {
                screen: Point::new(5.0, 5.0),
                delta: 0.1,
                phase: Phase::Ended,
            }],
        );
        assert_eq!(
            effects,
            vec![
                Effect::ZoomAbout {
                    anchor: Point::new(5.0, 5.0),
                    factor: 1.1,
                },
                Effect::GestureEnded,
            ]
        );
    }

    fn on(id: &str, region: NodeRegion) -> NodeHit {
        NodeHit {
            id: NodeId::from(id),
            region,
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
        }
    }

    fn press(at: Point, hit: NodeHit) -> Input {
        Input::Down {
            button: Button::Left,
            screen: at,
            modifiers: Modifiers::default(),
            on: Some(Hit::Node(hit)),
        }
    }

    #[test]
    fn a_body_click_selects_but_never_drags_or_marquees() {
        let mut state = Interaction::default();
        let selected = BTreeSet::new();

        let dragging = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                press(Point::new(10.0, 100.0), on("t1", NodeRegion::Body)),
                Input::Move {
                    screen: Point::new(80.0, 100.0),
                },
            ],
        );
        assert!(
            dragging.is_empty(),
            "the body owns the pointer: {dragging:?}"
        );
        assert!(
            state.marquee_screen_rect().is_none(),
            "and a body drag is not a marquee"
        );

        let clicked = drive(
            &mut Interaction::default(),
            &GestureConfig::default(),
            &selected,
            vec![
                press(Point::new(10.0, 100.0), on("t1", NodeRegion::Body)),
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(10.0, 100.0),
                },
            ],
        );
        assert_eq!(clicked, vec![Effect::SelectOnly(vec![id("t1")])]);
    }

    fn cmd_press(at: Point, on: Hit) -> Input {
        Input::Down {
            button: Button::Left,
            screen: at,
            modifiers: Modifiers {
                secondary: true,
                ..Modifiers::default()
            },
            on: Some(on),
        }
    }

    /// The secondary modifier turns the body into a drag handle, selecting the node first when
    /// it is not part of the selection already.
    #[test]
    fn cmd_dragging_a_body_selects_the_node_and_moves_it() {
        let mut state = Interaction::default();

        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                cmd_press(
                    Point::new(10.0, 100.0),
                    Hit::Node(on("t1", NodeRegion::Body)),
                ),
                Input::Move {
                    screen: Point::new(80.0, 100.0),
                },
            ],
        );
        assert_eq!(
            effects,
            vec![
                Effect::SelectOnly(vec![id("t1")]),
                // Zoom is 2.0 in the harness, so 70 screen px is 35 world units.
                Effect::TranslateNodes {
                    ids: vec![id("t1")],
                    delta: Point::new(35.0, 0.0),
                },
            ]
        );
        assert!(state.marquee_screen_rect().is_none());
    }

    #[test]
    fn cmd_dragging_a_selected_body_moves_the_whole_selection() {
        let mut state = Interaction::default();
        let selected = BTreeSet::from([id("t1"), id("t2")]);

        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                cmd_press(
                    Point::new(10.0, 100.0),
                    Hit::Node(on("t1", NodeRegion::Body)),
                ),
                Input::Move {
                    screen: Point::new(10.0, 140.0),
                },
            ],
        );
        assert_eq!(
            effects,
            vec![Effect::TranslateNodes {
                ids: vec![id("t1"), id("t2")],
                delta: Point::new(0.0, 20.0),
            }]
        );
    }

    /// An edge is not draggable with or without the modifier: it still only selects.
    #[test]
    fn cmd_dragging_an_edge_still_does_nothing() {
        let mut state = Interaction::default();

        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                cmd_press(Point::new(20.0, 20.0), Hit::Edge(EdgeId::from("a->b"))),
                Input::Move {
                    screen: Point::new(200.0, 200.0),
                },
            ],
        );
        assert!(effects.is_empty(), "{effects:?}");
        assert!(state.marquee_screen_rect().is_none());
    }

    #[test]
    fn dragging_a_corner_resizes_in_world_units() {
        let mut state = Interaction::default();
        let selected = BTreeSet::new();

        // Camera zoom is 2.0 in the harness, so 40 screen px is 20 world units.
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                press(
                    Point::new(298.0, 198.0),
                    on("t1", NodeRegion::Resize(Corner::BottomRight)),
                ),
                Input::Move {
                    screen: Point::new(338.0, 238.0),
                },
            ],
        );

        let Some(Effect::ResizeNode {
            id: resized,
            bounds,
        }) = effects.last()
        else {
            panic!("expected a resize, got {effects:?}");
        };
        assert_eq!(resized, &id("t1"));
        assert_eq!(bounds.size, Size::new(320.0, 220.0));
        assert_eq!(bounds.origin, Point::new(0.0, 0.0));
        assert!(matches!(state, Interaction::ResizingNode { .. }));
    }

    /// Every frame of a resize measures from the press anchor, so the anchor and the pointer
    /// must be in the same units. Comparing a world anchor against raw screen pixels made the
    /// node leap by the camera's pan on the second move and then grow without bound.
    #[test]
    fn a_resize_measures_from_the_press_under_a_panned_camera() {
        let camera = Camera {
            pan: Point::new(-500.0, -300.0),
            zoom: 2.0,
        };
        let mut state = Interaction::default();
        let config = GestureConfig::default();
        let selected = BTreeSet::new();
        // The bottom-right corner of the 300x200 node, then two moves of 40 screen px each.
        let inputs = [
            press(
                camera.world_to_screen(Point::new(298.0, 198.0)),
                on("t1", NodeRegion::Resize(Corner::BottomRight)),
            ),
            Input::Move {
                screen: camera.world_to_screen(Point::new(318.0, 218.0)),
            },
            Input::Move {
                screen: camera.world_to_screen(Point::new(338.0, 238.0)),
            },
        ];
        let effects: Vec<Effect> = inputs
            .into_iter()
            .flat_map(|input| reduce(&mut state, &config, camera, &selected, input))
            .collect();

        let Some(Effect::ResizeNode { bounds, .. }) = effects.last() else {
            panic!("expected a resize, got {effects:?}");
        };
        assert_eq!(bounds.origin, Point::new(0.0, 0.0));
        assert_eq!(bounds.size, Size::new(340.0, 240.0));
    }

    #[test]
    fn dragging_a_top_left_corner_moves_the_origin() {
        let mut state = Interaction::default();
        let selected = BTreeSet::new();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                press(
                    Point::new(2.0, 2.0),
                    on("t1", NodeRegion::Resize(Corner::TopLeft)),
                ),
                Input::Move {
                    screen: Point::new(42.0, 42.0),
                },
            ],
        );

        let Some(Effect::ResizeNode { bounds, .. }) = effects.last() else {
            panic!("expected a resize, got {effects:?}");
        };
        assert_eq!(bounds.origin, Point::new(20.0, 20.0));
        assert_eq!(bounds.size, Size::new(280.0, 180.0));
    }

    #[test]
    fn resizing_still_works_while_the_camera_is_locked() {
        let mut state = Interaction::default();
        let locked = GestureConfig {
            camera_locked: true,
            space_held: false,
        };
        let effects = drive(
            &mut state,
            &locked,
            &BTreeSet::new(),
            vec![
                press(
                    Point::new(298.0, 198.0),
                    on("t1", NodeRegion::Resize(Corner::BottomRight)),
                ),
                Input::Move {
                    screen: Point::new(318.0, 198.0),
                },
            ],
        );
        assert!(matches!(effects.last(), Some(Effect::ResizeNode { .. })));
    }

    #[test]
    fn a_placement_click_centres_a_default_sized_node_on_the_cursor() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                Input::ArmTool(Some(NodeType::Text)),
                Input::Down {
                    button: Button::Left,
                    screen: Point::new(100.0, 100.0),
                    modifiers: Modifiers::default(),
                    on: None,
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(100.0, 100.0),
                },
            ],
        );

        let size = NodeType::Text.default_size();
        // Zoom 2.0, pan 0: screen (100, 100) is world (50, 50).
        assert_eq!(
            effects,
            vec![
                Effect::PlaceNode {
                    node_type: NodeType::Text,
                    world: Rect::new(
                        Point::new(50.0 - size.width / 2.0, 50.0 - size.height / 2.0),
                        size,
                    ),
                },
                Effect::CommitPlacement,
            ]
        );
    }

    #[test]
    fn a_placement_drag_uses_the_dragged_rect_clamped_to_min_size() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                Input::ArmTool(Some(NodeType::Text)),
                Input::Down {
                    button: Button::Left,
                    screen: Point::new(0.0, 0.0),
                    modifiers: Modifiers::default(),
                    on: None,
                },
                Input::Move {
                    screen: Point::new(600.0, 40.0),
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(600.0, 40.0),
                },
            ],
        );

        let Some(Effect::PlaceNode { world, .. }) = effects.first() else {
            panic!("expected a placement, got {effects:?}");
        };
        assert_eq!(
            world.size,
            Size::new(300.0, NodeType::Text.min_size().height),
            "600 screen px at zoom 2, and 20 world units clamped up to the minimum"
        );
        assert_eq!(effects.last(), Some(&Effect::CommitPlacement));
    }

    /// The point of placing on the first move rather than on the release: the node exists while
    /// the pointer is still down, so the drag resizes the real thing.
    #[test]
    fn a_placement_drag_creates_the_node_once_and_resizes_it_while_it_runs() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                Input::ArmTool(Some(NodeType::Text)),
                Input::Down {
                    button: Button::Left,
                    screen: Point::new(0.0, 0.0),
                    modifiers: Modifiers::default(),
                    on: None,
                },
                // Inside the threshold: still nothing on the canvas.
                Input::Move {
                    screen: Point::new(2.0, 0.0),
                },
                Input::Move {
                    screen: Point::new(600.0, 400.0),
                },
                Input::Move {
                    screen: Point::new(800.0, 400.0),
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(800.0, 400.0),
                },
            ],
        );

        assert_eq!(
            effects,
            vec![
                Effect::PlaceNode {
                    node_type: NodeType::Text,
                    world: Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
                },
                Effect::ResizePlacement {
                    world: Rect::new(Point::new(0.0, 0.0), Size::new(400.0, 200.0)),
                },
                Effect::CommitPlacement,
            ]
        );
        assert!(state.is_idle(), "the tool disarms once the node is placed");
    }

    /// Camera in [`drive`]: zoom 2, pan 0, so screen `(x, y)` is world `(x / 2, y / 2)`.
    fn stroke(state: &mut Interaction, screens: &[Point]) -> Vec<Effect> {
        let (first, rest) = screens.split_first().expect("a stroke has a press");
        let mut inputs = vec![down(Button::Left, *first, None)];
        inputs.extend(rest.iter().map(|screen| Input::Move { screen: *screen }));
        inputs.push(Input::Up {
            button: Button::Left,
            screen: *screens.last().expect("non-empty"),
        });
        drive(state, &GestureConfig::default(), &BTreeSet::new(), inputs)
    }

    #[test]
    fn the_draw_tool_arms_into_its_own_state_and_stays_there() {
        let mut state = Interaction::default();
        drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![Input::ArmTool(Some(NodeType::Draw))],
        );

        assert!(matches!(state, Interaction::Drawing { .. }));
        assert_eq!(
            state.armed_tool(),
            Some(NodeType::Draw),
            "so the toolbar button lights up"
        );

        let effects = stroke(
            &mut state,
            &[
                Point::new(100.0, 100.0),
                Point::new(140.0, 120.0),
                Point::new(180.0, 100.0),
            ],
        );

        assert_eq!(
            effects,
            vec![Effect::PlaceDrawing {
                points: vec![
                    Point::new(50.0, 50.0),
                    Point::new(70.0, 60.0),
                    Point::new(90.0, 50.0),
                ],
            }],
            "every move is a sample, in world units"
        );
        // Sticky, unlike every other place tool: `useDrawTool` never clears place mode.
        assert!(matches!(state, Interaction::Drawing { samples } if samples.is_empty()));
    }

    #[test]
    fn a_stroke_started_over_a_node_draws_instead_of_dragging_it() {
        let mut state = Interaction::default();
        let mut selected = BTreeSet::new();
        selected.insert(NodeId::from("t1"));
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &selected,
            vec![
                Input::ArmTool(Some(NodeType::Draw)),
                press(Point::new(10.0, 10.0), on("t1", NodeRegion::Header)),
                Input::Move {
                    screen: Point::new(90.0, 10.0),
                },
                Input::Up {
                    button: Button::Left,
                    screen: Point::new(90.0, 10.0),
                },
            ],
        );

        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::TranslateNodes { .. })),
            "the header is a drag handle only when the pen is away: {effects:?}"
        );
        assert!(matches!(effects.as_slice(), [Effect::PlaceDrawing { .. }]));
    }

    #[test]
    fn a_click_draws_nothing_and_leaves_the_tool_armed() {
        let mut state = Interaction::default();
        drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![Input::ArmTool(Some(NodeType::Draw))],
        );

        let effects = stroke(&mut state, &[Point::new(40.0, 40.0)]);

        assert_eq!(effects, Vec::new(), "one sample is not a stroke");
        assert!(matches!(state, Interaction::Drawing { .. }));
    }

    #[test]
    fn a_wheel_still_pans_while_the_pen_is_armed_but_other_buttons_do_nothing() {
        let mut state = Interaction::default();
        let effects = drive(
            &mut state,
            &GestureConfig::default(),
            &BTreeSet::new(),
            vec![
                Input::ArmTool(Some(NodeType::Draw)),
                down(Button::Middle, Point::new(10.0, 10.0), None),
                Input::Move {
                    screen: Point::new(60.0, 10.0),
                },
                Input::Up {
                    button: Button::Middle,
                    screen: Point::new(60.0, 10.0),
                },
                Input::Wheel {
                    screen: Point::new(10.0, 10.0),
                    delta: Point::new(0.0, -30.0),
                    modifiers: Modifiers::default(),
                    phase: Phase::Moved,
                },
            ],
        );

        assert_eq!(
            effects,
            vec![Effect::Pan(Point::new(0.0, -30.0))],
            "the middle drag is ignored and the wheel falls through: {effects:?}"
        );
        assert!(matches!(state, Interaction::Drawing { .. }), "still armed");
    }

    const SCROLL_MAX: f64 = 1000.0;

    #[test]
    fn a_scroller_short_enough_to_fit_has_no_room_either_way() {
        assert!(!has_room(-120.0, 0.0, 0.0));
        assert!(!has_room(120.0, 0.0, 0.0));
    }

    #[test]
    fn a_half_scrolled_scroller_has_room_either_way() {
        assert!(has_room(-120.0, -500.0, SCROLL_MAX));
        assert!(has_room(120.0, -500.0, SCROLL_MAX));
    }

    /// At the start, scrolling back has nowhere to go and the canvas should pan instead — this
    /// is what keeps a node body from trapping the gesture.
    #[test]
    fn at_the_start_only_the_forward_direction_has_room() {
        assert!(!has_room(120.0, 0.0, SCROLL_MAX));
        assert!(has_room(-120.0, 0.0, SCROLL_MAX));
    }

    #[test]
    fn at_the_end_only_the_backward_direction_has_room() {
        assert!(!has_room(-120.0, -SCROLL_MAX, SCROLL_MAX));
        assert!(has_room(120.0, -SCROLL_MAX, SCROLL_MAX));
    }

    /// A fraction of a pixel of travel is still travel. The rule carries no epsilon on purpose:
    /// it has to agree with the scrollers it arbitrates for, and they have none either, so a
    /// tolerance here would hand the canvas a wheel the body was about to use.
    #[test]
    fn a_fraction_of_a_pixel_of_room_is_still_room() {
        assert!(has_room(120.0, -0.2, SCROLL_MAX));
        assert!(has_room(-120.0, -999.8, SCROLL_MAX));
    }

    /// The reason both sides are clamped: a wheel writes the offset unclamped and only the next
    /// layout pulls it back, so between the two the offset reads past the edge. Without the
    /// clamp that overscroll would look like room and the body would absorb for ever.
    #[test]
    fn a_transient_overscroll_past_the_end_still_reads_as_the_end() {
        assert!(!has_room(-120.0, -SCROLL_MAX - 400.0, SCROLL_MAX));
        assert!(!has_room(120.0, 400.0, SCROLL_MAX));
    }

    #[test]
    fn arming_a_tool_and_cancelling_leaves_no_placement() {
        let mut state = Interaction::default();
        reduce(
            &mut state,
            &GestureConfig::default(),
            Camera::default(),
            &BTreeSet::new(),
            Input::ArmTool(Some(NodeType::Variable)),
        );
        assert_eq!(state.armed_tool(), Some(NodeType::Variable));

        reduce(
            &mut state,
            &GestureConfig::default(),
            Camera::default(),
            &BTreeSet::new(),
            Input::ArmTool(None),
        );
        assert!(state.is_idle());
        assert_eq!(state.armed_tool(), None);
    }
}
