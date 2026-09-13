//! Adding and editing a connection, ported from `~/labs/peek/src/Connection/ConnectionForm.tsx`
//! and `SshTunnelFields.tsx`.
//!
//! There is no separate driver, host, port, user or password field, as there is none in the
//! reference: one URL carries all of it, and the engine is derived from its scheme. What the
//! form adds under it is a colour-coded preview of the parts, so a typo in a masked string is
//! still visible.
//!
//! One thing here is ours: **Test connection**. The reference has no way to check a connection
//! without switching to it, which means finding out a password is wrong by losing the canvas
//! you were looking at.

use gpui_kit::AppContext;
use gpui_kit::base::input::InputState;
use gpui_kit::{App, Context, Entity, SharedString, Window};
use peek_config::{DatabaseConnection, PersistenceMode, SshTunnelConfig, UrlParts, WorkspaceError};
use peek_db::Engine;
use peek_document::DocumentStore;

use super::PickerView;
use crate::settings::Settings;

/// The default a new tunnel starts from, and the reference's own defaults.
const DEFAULT_SSH_PORT: u16 = 22;
const DEFAULT_LOCAL_PORT: u16 = 15432;

/// How the Test button is getting on. It never touches the `Database` global: a probe must not
/// disturb what the canvas is querying, so its result lives here and nowhere else.
#[derive(Debug, Default)]
pub(super) enum Probe {
    #[default]
    Idle,
    Running,
    Reached(Engine),
    Failed(SharedString),
}

/// The SSH sub-form. Ports are text rather than `NumberInput` because a half-typed port is a
/// normal state while someone is editing one, and a number field has nowhere to put "".
pub(super) struct TunnelFields {
    pub(super) host: Entity<InputState>,
    pub(super) user: Entity<InputState>,
    pub(super) key_path: Entity<InputState>,
    pub(super) ssh_port: Entity<InputState>,
    pub(super) local_port: Entity<InputState>,
}

impl TunnelFields {
    pub(super) fn new(
        config: Option<&SshTunnelConfig>,
        window: &mut Window,
        cx: &mut Context<PickerView>,
    ) -> Self {
        let field = |value: String, window: &mut Window, cx: &mut Context<PickerView>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let ports = config.map_or((DEFAULT_SSH_PORT, DEFAULT_LOCAL_PORT), |tunnel| {
            (tunnel.ssh_port, tunnel.local_port)
        });
        Self {
            host: field(text(config, |tunnel| tunnel.ssh_host.clone()), window, cx),
            user: field(text(config, |tunnel| tunnel.ssh_user.clone()), window, cx),
            key_path: field(
                text(config, |tunnel| {
                    tunnel.key_path.to_string_lossy().to_string()
                }),
                window,
                cx,
            ),
            ssh_port: field(ports.0.to_string(), window, cx),
            local_port: field(ports.1.to_string(), window, cx),
        }
    }

    /// The tunnel as configured, or `None` when the host is blank — an SSH switch turned on and
    /// left empty is not a tunnel, and writing one would fail at connect time instead of here.
    fn collect(&self, cx: &App) -> Option<SshTunnelConfig> {
        let value = |field: &Entity<InputState>| field.read(cx).value().trim().to_string();
        let host = value(&self.host);
        if host.is_empty() {
            return None;
        }
        Some(SshTunnelConfig {
            ssh_host: host,
            ssh_user: value(&self.user),
            key_path: value(&self.key_path).into(),
            ssh_port: value(&self.ssh_port).parse().unwrap_or(DEFAULT_SSH_PORT),
            local_port: value(&self.local_port)
                .parse()
                .unwrap_or(DEFAULT_LOCAL_PORT),
        })
    }
}

fn text(config: Option<&SshTunnelConfig>, read: impl Fn(&SshTunnelConfig) -> String) -> String {
    config.map(read).unwrap_or_default()
}

/// The live form. `editing` is what separates "add" from "edit", and it holds the name the
/// connection had when the form opened — which is what a rename has to move files away from.
pub(super) struct ConnectionForm {
    pub(super) workspace: SharedString,
    pub(super) editing: Option<SharedString>,
    pub(super) name: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) color: String,
    pub(super) tunnel: Option<TunnelFields>,
    pub(super) probe: Probe,
    /// Why the last save did not happen. Cleared when the form is next submitted.
    pub(super) error: Option<SharedString>,
    pub(super) confirming_remove: bool,
}

impl ConnectionForm {
    /// A form over an existing connection, or a blank one when `editing` names nothing.
    pub(super) fn new(
        at: (&str, Option<&str>),
        window: &mut Window,
        cx: &mut Context<PickerView>,
    ) -> Self {
        let (workspace, editing) = at;
        let existing = editing.and_then(|name| Settings::get(cx).connection((workspace, name)));
        let color = existing.map_or_else(
            || peek_config::CONNECTION_COLOR_PRESETS[0].1.to_string(),
            |connection| connection.color.clone(),
        );
        let name_value = existing.map(|connection| connection.name.clone());
        let url_value = existing.map(|connection| connection.url.clone());
        let tunnel = existing.and_then(|connection| connection.ssh_tunnel.clone());

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. staging")
                .default_value(name_value.unwrap_or_default())
        });
        // Masked, as the reference's `type={reveal ? "text" : "password"}` is: the URL carries
        // the password, and a connection string is routinely read over someone's shoulder.
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("postgres://user:password@host:port/database")
                .default_value(url_value.unwrap_or_default())
                .masked(true)
        });

        Self {
            workspace: SharedString::from(workspace.to_string()),
            editing: editing.map(|name| SharedString::from(name.to_string())),
            name,
            url,
            color,
            tunnel: tunnel
                .as_ref()
                .map(|config| TunnelFields::new(Some(config), window, cx)),
            probe: Probe::Idle,
            error: None,
            confirming_remove: false,
        }
    }

    pub(super) fn set_color(&mut self, color: &str) {
        self.color = color.to_string();
    }

    /// The URL as typed, split for the preview. Parses leniently and on every keystroke, so a
    /// half-written URL still shows the parts it has.
    pub(super) fn parts(&self, cx: &App) -> Option<UrlParts> {
        DatabaseConnection {
            url: self.url.read(cx).value().to_string(),
            ..DatabaseConnection::default()
        }
        .parts()
    }

    /// The connection as the form currently describes it.
    pub(super) fn collect(&self, cx: &App) -> DatabaseConnection {
        DatabaseConnection {
            name: self.name.read(cx).value().trim().to_string(),
            color: self.color.clone(),
            url: self.url.read(cx).value().trim().to_string(),
            ssh_tunnel: self.tunnel.as_ref().and_then(|fields| fields.collect(cx)),
        }
    }

    /// Whether Save would do anything: the reference's `name && url`.
    pub(super) fn can_save(&self, cx: &App) -> bool {
        !self.name.read(cx).value().trim().is_empty()
            && !self.url.read(cx).value().trim().is_empty()
    }
}

/// What a committed form did, so the workspace can follow a connection it has open.
pub(super) enum Committed {
    Added,
    Updated,
    /// The name changed, and the document has already been moved to match.
    Renamed {
        from: String,
        to: String,
    },
}

/// Writes the form to `settings.json`, moving the canvas first when the name changed.
///
/// Order is load-bearing. The files move **before** the config commit, so a failed move leaves
/// `settings.json` still naming a connection whose document is where it says it is — the
/// reverse would leave a config entry pointing at nothing.
pub(super) fn commit(
    form: &ConnectionForm,
    mode: PersistenceMode,
    cx: &mut App,
) -> Result<Committed, SharedString> {
    let connection = form.collect(cx);
    let workspace = form.workspace.to_string();

    let Some(previous) = form.editing.clone() else {
        write(cx, |config| config.add_connection(&workspace, connection))?;
        return Ok(Committed::Added);
    };

    let renamed = !previous.eq_ignore_ascii_case(&connection.name);
    let new_name = connection.name.clone();
    if renamed {
        move_document(&workspace, (&previous, &new_name), mode)?;
    }

    let at = previous.to_string();
    write(cx, |config| {
        config.update_connection((&workspace, &at), connection)
    })?;

    if renamed {
        return Ok(Committed::Renamed {
            from: previous.to_string(),
            to: new_name,
        });
    }
    Ok(Committed::Updated)
}

/// Applies a `WorkspaceError`-returning edit and saves, flattening both failures to something a
/// form can show. The in-memory edit stands even when the write does not, which is what
/// `PersistenceMode::ReadOnly` means.
pub(super) fn write(
    cx: &mut App,
    change: impl FnOnce(&mut peek_config::PeekConfig) -> Result<(), WorkspaceError>,
) -> Result<(), SharedString> {
    let mut refused: Option<WorkspaceError> = None;
    let saved = Settings::update(cx, |config| {
        if let Err(error) = change(config) {
            refused = Some(error);
        }
    });
    if let Some(error) = refused {
        return Err(SharedString::from(error.to_string()));
    }
    saved.map_err(|error| SharedString::from(error.to_string()))
}

/// Moves a renamed connection's document and rows sidecar, so the canvas follows its name.
/// The reference leaves both behind and opens the renamed connection on an empty board.
fn move_document(
    workspace: &str,
    names: (&str, &str),
    mode: PersistenceMode,
) -> Result<(), SharedString> {
    if !mode.can_write() {
        return Ok(());
    }
    let store = DocumentStore::new(mode).map_err(|error| SharedString::from(error.to_string()))?;
    store
        .rename(workspace, names.0, names.1)
        .map_err(|error| SharedString::from(format!("could not move the canvas: {error}")))
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, TestSupportExt, div, px};
use peek_theme::ActivePeekTheme;

use super::View;

/// The form, filling the panel below its own header.
pub(super) fn body(form: &ConnectionForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (fg, border) = (theme.fg, theme.node_border);
    let adding = form.editing.is_none();
    let title = form.editing.clone().map_or_else(
        || SharedString::from(format!("New connection · in {}", form.workspace)),
        |name| SharedString::from(format!("{name} · {}", form.workspace)),
    );

    div()
        .id("connection-form")
        .test_support()
        .v_flex()
        .flex_1()
        .min_h_0()
        .child(super::list::form_header(title, cx))
        .child(
            div()
                .id("connection-form-fields")
                .v_flex()
                .gap(px(12.0))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(18.0))
                .py(px(14.0))
                .child(field("Name", Input::new(&form.name).small(), cx))
                .child(swatches(&form.color, cx))
                .child(field("Connection URL", url_field(form), cx))
                .child(preview(form, cx))
                .child(tunnel_section(form, cx))
                .children(form.error.clone().map(|message| strip(message, cx))),
        )
        .child(
            div()
                .h_flex()
                .justify_between()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(border)
                .child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .children((!adding).then(|| remove_control(form, cx))),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .child(test_control(form, cx))
                        .child(
                            Button::new("connection-cancel")
                                .ghost()
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.back_to_list(window, cx);
                                })),
                        )
                        .child(save_control(form, adding, cx)),
                ),
        )
        .text_color(fg)
        .text_size(px(12.5))
}

/// A labelled row. The label is above the control, as the reference's `.picker-field` is.
fn field(
    label: &'static str,
    control: impl IntoElement,
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .v_flex()
        .gap(px(5.0))
        .child(
            div()
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.fg_subtle)
                .child(label),
        )
        .child(control)
}

/// The URL is masked by default, with the eye toggle the reference's reveal button is.
fn url_field(form: &ConnectionForm) -> impl IntoElement {
    Input::new(&form.url).small().mask_toggle()
}

/// The six presets, each writing its own literal string back. A seventh swatch for a custom
/// colour is not ported: gpui-component's picker speaks `Hsla`, and round-tripping through it
/// would rewrite the two `hsl(...)` presets as hex in a file the TypeScript app also reads.
fn swatches(current: &str, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (ring, ground) = (theme.fg, theme.node_bg);
    let mut row = div().h_flex().gap(px(8.0));
    for (name, color) in peek_config::CONNECTION_COLOR_PRESETS {
        let swatch = super::super::connection::resolve(
            DatabaseConnection {
                color: color.to_string(),
                ..DatabaseConnection::default()
            }
            .rgb(),
        )
        .unwrap_or(theme.fg_subtle);
        let active = current == color;
        row = row.child(
            div()
                .id(SharedString::from(format!("connection-color-{name}")))
                .test_support()
                .size(px(22.0))
                .rounded_full()
                .bg(swatch)
                .when(active, |this| {
                    // Two rings, the outer in the panel's own ground, so the marker reads on a
                    // swatch of any colour.
                    this.border_2()
                        .border_color(ground)
                        .shadow(vec![gpui_kit::BoxShadow {
                            color: ring,
                            offset: gpui_kit::point(px(0.0), px(0.0)),
                            blur_radius: px(0.0),
                            spread_radius: px(1.0),
                            inset: false,
                        }])
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(View::Connection(form)) = this.view_mut() {
                        form.set_color(color);
                    }
                    cx.notify();
                })),
        );
    }
    field("Colour", row, cx)
}

/// The URL's parts, colour-coded. The reference's `.picker-url-preview`, and the reason a
/// masked field is still checkable by eye.
fn preview(form: &ConnectionForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (subtle, green, yellow, accent) =
        (theme.fg_subtle, theme.green, theme.yellow, theme.accent);
    let Some(parts) = form.parts(cx) else {
        return div();
    };
    let port = parts
        .port
        .map_or_else(String::new, |port| format!(":{port}"));

    div()
        .h_flex()
        .font_family("Monaspace Krypton")
        .text_size(px(11.0))
        .truncate()
        .child(
            div()
                .text_color(subtle)
                .child(format!("{}://", parts.scheme)),
        )
        .child(div().text_color(green).child(parts.user.clone()))
        .when(!parts.user.is_empty(), |this| {
            this.child(div().text_color(subtle).child("@"))
        })
        .child(
            div()
                .text_color(yellow)
                .child(format!("{}{port}", parts.host)),
        )
        .when(!parts.database.is_empty(), |this| {
            this.child(
                div()
                    .text_color(accent)
                    .child(format!("/{}", parts.database)),
            )
        })
}

/// The SSH switch and, when it is on, the five fields behind it.
fn tunnel_section(form: &ConnectionForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let border = cx.peek_theme().node_border;
    let on = form.tunnel.is_some();

    div()
        .v_flex()
        .gap(px(10.0))
        .child(
            div().h_flex().justify_between().items_center().child(
                Switch::new("connection-ssh-toggle")
                    .checked(on)
                    .label("SSH tunnel")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(View::Connection(_)) = this.view() {
                            this.toggle_tunnel(window, cx);
                        }
                    })),
            ),
        )
        .children(form.tunnel.as_ref().map(|fields| {
            div()
                .v_flex()
                .gap(px(10.0))
                .pl(px(10.0))
                .border_l_1()
                .border_color(border)
                .child(field("SSH host", Input::new(&fields.host).small(), cx))
                .child(field("SSH user", Input::new(&fields.user).small(), cx))
                .child(field(
                    "Identity key",
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .child(div().flex_1().child(Input::new(&fields.key_path).small()))
                        .child(browse_button(cx)),
                    cx,
                ))
                .child(
                    div()
                        .h_flex()
                        .gap(px(10.0))
                        .child(div().flex_1().child(field(
                            "SSH port",
                            Input::new(&fields.ssh_port).small(),
                            cx,
                        )))
                        .child(div().flex_1().child(field(
                            "Local port",
                            Input::new(&fields.local_port).small(),
                            cx,
                        ))),
                )
        }))
}

/// Opens the platform file chooser and writes the chosen path into the key field.
fn browse_button(cx: &mut Context<PickerView>) -> impl IntoElement {
    Button::new("ssh-browse")
        .outline()
        .xsmall()
        .icon(Icon::new(IconName::Key))
        .label("Browse\u{2026}")
        .on_click(cx.listener(|_, _, window, cx| PickerView::browse_for_key(window, cx)))
}

/// Test connection. Disabled until there is a URL to test, and never routed through the
/// `Database` global — the canvas keeps the connection it has.
fn test_control(form: &ConnectionForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (label, color) = match &form.probe {
        Probe::Idle => (SharedString::from("Test"), theme.fg_muted),
        Probe::Running => (SharedString::from("Testing\u{2026}"), theme.fg_muted),
        Probe::Reached(engine) => (SharedString::from(format!("Reached {engine}")), theme.green),
        Probe::Failed(_) => (SharedString::from("Failed"), theme.red),
    };
    let tooltip = match &form.probe {
        Probe::Failed(message) => Some(message.clone()),
        _ => None,
    };
    let ready = !form.url.read(cx).value().trim().is_empty();

    Button::new("connection-test")
        .ghost()
        .xsmall()
        .label(label)
        .text_color(color)
        .disabled(!ready || matches!(form.probe, Probe::Running))
        .when_some(tooltip, Button::tooltip)
        .on_click(cx.listener(|this, _, window, cx| this.probe_connection(window, cx)))
}

/// Save, disabled with a reason when this run may not write — the rule every save path here
/// follows.
fn save_control(
    form: &ConnectionForm,
    adding: bool,
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let writable = Settings::can_write(cx);
    let enabled = writable && form.can_save(cx);
    let label = if adding { "Add connection" } else { "Save" };

    Button::new("connection-save")
        .primary()
        .xsmall()
        .label(label)
        .disabled(!enabled)
        .when(!writable, |button| {
            button.tooltip("Read-only: relaunch with --write to change settings.json")
        })
        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
}

/// Remove, behind an inline confirm rather than a second dialog: the panel is already an
/// overlay, and stacking one on it is what the design guide tells you not to do.
fn remove_control(form: &ConnectionForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    if !form.confirming_remove {
        return div().child(
            Button::new("connection-remove")
                .ghost()
                .xsmall()
                .label("Remove")
                .text_color(theme.red)
                .on_click(cx.listener(|this, _, _, cx| this.arm_remove(cx))),
        );
    }

    let name = form.editing.clone().unwrap_or_default();
    div()
        .h_flex()
        .gap(px(6.0))
        .items_center()
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.fg_subtle)
                .child(SharedString::from(format!(
                    "Remove \u{201c}{name}\u{201d}?"
                ))),
        )
        .child(
            Button::new("connection-remove-confirm")
                .danger()
                .xsmall()
                .label("Remove")
                .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
        )
}

/// Why the last save did not happen, under the fields rather than floating over them.
fn strip(message: SharedString, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .gap(px(8.0))
        .items_start()
        .p(px(8.0))
        .rounded(px(6.0))
        .bg(theme.red_soft)
        .text_size(px(11.5))
        .text_color(theme.red)
        .child(Icon::new(IconName::TriangleAlert).size(px(11.0)))
        .child(div().min_w_0().child(message))
}
