//! Every built-in must be a complete, self-consistent theme.

use gpui_kit::Rgba;
use gpui_kit::component::theme::ThemeMode;
use peek_config::ThemeId;
use peek_theme::{PeekTheme, builtin, to_component_config};

#[test]
fn ids_match_and_every_theme_resolves() {
    for id in ThemeId::ALL {
        let spec = builtin::spec(id);
        assert_eq!(spec.id, id);
        let resolved = PeekTheme::from_spec(spec);
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.name.as_ref(), spec.name);
    }
    assert_eq!(builtin::all().count(), ThemeId::ALL.len());
}

#[test]
fn mode_matches_lightness() {
    for spec in builtin::all() {
        let config = to_component_config(spec);
        assert_eq!(
            config.mode == ThemeMode::Light,
            spec.is_light,
            "{}",
            spec.name
        );
        assert_eq!(spec.bg.luminance() > 0.5, spec.is_light, "{}", spec.name);
    }
}

#[test]
fn required_component_roles_are_set_and_parse() {
    for spec in builtin::all() {
        let config = to_component_config(spec);
        let colors = &config.colors;
        let required = [
            ("background", &colors.background),
            ("foreground", &colors.foreground),
            ("border", &colors.border),
            ("ring", &colors.ring),
            ("primary", &colors.primary),
            ("primary_foreground", &colors.primary_foreground),
            ("secondary", &colors.secondary),
            ("popover", &colors.popover),
            ("muted", &colors.muted),
            ("muted_foreground", &colors.muted_foreground),
            ("list_active", &colors.list_active),
            ("danger", &colors.danger),
            ("title_bar", &colors.title_bar),
        ];
        for (name, value) in required {
            let value = value
                .as_ref()
                .unwrap_or_else(|| panic!("{}: {name} unset", spec.name));
            Rgba::try_from(value.as_ref())
                .unwrap_or_else(|error| panic!("{}: {name} {value}: {error:?}", spec.name));
        }
        assert_eq!(config.font_family.as_deref(), Some("Monaspace Krypton"));
    }
}

#[test]
fn text_stays_readable() {
    for spec in builtin::all() {
        assert!(spec.fg.contrast(spec.bg) >= 4.5, "{}: fg on bg", spec.name);
        assert!(
            spec.fg.contrast(spec.node_bg) >= 4.5,
            "{}: fg on node",
            spec.name
        );
        assert!(
            spec.fg_muted.contrast(spec.node_bg) >= 3.0,
            "{}: muted on node",
            spec.name
        );
    }
}

/// The categorical hues are label colours, so they are held to the muted-text floor rather
/// than the 4.5 body-text one. Worth asserting: the TypeScript app uses one hardcoded pink and
/// teal for these in every stylesheet, which lands at 2.7:1 and 2.1:1 on the light themes.
#[test]
fn categorical_hues_stay_legible_on_nodes() {
    for spec in builtin::all() {
        assert!(
            spec.magenta.contrast(spec.node_bg) >= 3.0,
            "{}: magenta on node is {:.2}",
            spec.name,
            spec.magenta.contrast(spec.node_bg)
        );
        assert!(
            spec.cyan.contrast(spec.node_bg) >= 3.0,
            "{}: cyan on node is {:.2}",
            spec.name,
            spec.cyan.contrast(spec.node_bg)
        );
    }
}

#[test]
fn selected_rows_move_away_from_the_background() {
    for spec in builtin::all() {
        let (selected, hovered) = spec.row_selected();
        let direction = |color: peek_theme::Color| color.luminance() - spec.node_bg.luminance();
        if spec.is_light {
            assert!(direction(selected) < 0.0 && direction(hovered) < direction(selected));
        } else {
            assert!(direction(selected) > 0.0 && direction(hovered) > direction(selected));
        }
    }
}

/// The SQL editor reads its colours from `ThemeConfig::highlight`, not from `PeekTheme`, so a
/// theme that forgets it renders code in one flat foreground.
#[test]
fn every_theme_carries_syntax_colours_for_the_editor() {
    for spec in builtin::all() {
        let config = to_component_config(spec);
        let highlight = config
            .highlight
            .as_ref()
            .unwrap_or_else(|| panic!("{}: no highlight style", spec.name));

        for role in ["keyword", "string", "number", "comment", "type", "variable"] {
            let style = highlight
                .syntax
                .style(role)
                .unwrap_or_else(|| panic!("{}: syntax role {role} unset", spec.name));
            assert!(
                style.color.is_some(),
                "{}: syntax role {role} has no colour",
                spec.name
            );
        }

        let foreground = highlight
            .editor_foreground
            .unwrap_or_else(|| panic!("{}: editor.foreground unset", spec.name));
        let comment = highlight
            .syntax
            .style("comment")
            .and_then(|style| style.color)
            .expect("comment colour");
        assert_ne!(
            comment, foreground,
            "{}: comments must not vanish into body text",
            spec.name
        );

        // The editor sits on a node card; matching the canvas background would cut a hole in it.
        let background = highlight.editor_background.map_or_else(
            || panic!("{}: editor.background unset", spec.name),
            Rgba::from,
        );
        let node_bg = Rgba::from(PeekTheme::from_spec(spec).node_bg);
        assert!(
            (background.r - node_bg.r).abs() < 1.5 / 255.0
                && (background.g - node_bg.g).abs() < 1.5 / 255.0
                && (background.b - node_bg.b).abs() < 1.5 / 255.0,
            "{}: editor.background should be the node surface",
            spec.name
        );
    }
}

/// `keyword_control` is carried on the `constant` role because SQL emits no `@constant`; the
/// other half of that pairing is `peek_lsp::sql_highlights`. If either side moves, the second
/// keyword hue silently collapses into `keyword`.
#[test]
fn the_second_keyword_hue_is_carried_on_constant() {
    for spec in builtin::all() {
        let config = to_component_config(spec);
        let syntax = &config.highlight.as_ref().expect("highlight").syntax;
        let carried = syntax
            .style("constant")
            .and_then(|style| style.color)
            .unwrap_or_else(|| panic!("{}: constant unset", spec.name));
        let expected = Rgba::try_from(spec.syntax.keyword_control.hex().as_str()).expect("hex");
        let carried = Rgba::from(carried);
        // Within one 8-bit step: the hex round-trips through f32 HSL on the way in.
        let step = 1.5 / 255.0;
        for (channel, (got, want)) in [
            ("r", (carried.r, expected.r)),
            ("g", (carried.g, expected.g)),
            ("b", (carried.b, expected.b)),
        ] {
            assert!(
                (got - want).abs() < step,
                "{}: constant should carry keyword_control, {channel} {got} != {want}",
                spec.name
            );
        }
    }
}
