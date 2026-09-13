# Themes

## Model

Two globals, always applied together by `peek_theme::ThemeService`:

1. gpui-component's `Theme` (139 colour roles read by every widget) — fed by
   `to_component_config(spec) -> Rc<ThemeConfig>` and applied with
   `Theme::global_mut(cx).apply_config(&cfg)` followed by `Theme::change(mode, None, cx)`.
   `apply_config` stores the config in the light/dark slot that `change` re-applies, then
   `change` resolves the mono font and pushes the projection to the Base layer (scrollbars).
2. `PeekTheme` (Peek's canvas roles as `Hsla`/`Pixels`), reachable as `cx.peek_theme()` through
   `ActivePeekTheme`, mirroring `cx.theme()`.

Rule: application code never uses a raw colour. Widgets read `cx.theme()`, canvas and nodes read
`cx.peek_theme()`.

## Authoring: `ThemeSpec`

`crates/peek-theme/src/spec.rs` defines `ThemeSpec`, a `const`-constructible record with a
`Color(u32)` newtype (`0xRRGGBBAA`). One table per built-in under `src/builtin/`:

| Id | Source palette | Notes |
|---|---|---|
| `pine` (default) | oklch → sRGB | radial glows in the CSS canvas background dropped |
| `midnight` | oklch → sRGB | |
| `midday` | oklch → sRGB | `--pk-bg-grid` chroma 0.8 in the CSS is an out-of-gamut typo; 0.08 used |
| `terminal` | hex | zero radius, corner brackets, tick indicators, phosphor `active` selection, mono chart ramp |
| `paper` | hex (Rosé Pine Dawn) | light, 14/12 px radii |
| `blueprint` | hex (Rosé Pine Moon) | 2 px radii, cyan brackets, tick indicators |

Fields: surfaces (`bg`, `bg_grid`, `canvas { base, gradient }`, `node_bg/_2/_inset`,
`node_border/_strong`, `node_shadow`), text ramp (`fg`, `fg_muted`, `fg_subtle`), accent set,
optional `active`, `row_selected_mix`, four status hues with `_soft` variants, `regions[5]`,
`chart_series[5]`, `node_types` (query/agent/result/chart/error/variable/activity), radii,
`node_frame` (`Plain` or `Brackets`), `type_indicator` (`Dot` or `Tick`) and `syntax` (kept as
data for the editor milestone). The original CSS literal follows each converted colour as a
comment. `builtin::spec(ThemeId)` is an exhaustive match, so a new `ThemeId` variant fails to
compile until a table exists.

oklch conversion was done once with a scratch script implementing the CSS Color 4 oklch→sRGB
math with clipping. Radial gradients have no gpui equivalent; only the vertical linear gradient
is kept, painted over the base fill.

## Mapping onto gpui-component

`component_map.rs` builds `ThemeConfigColors` from dotted JSON keys (the `base.*` fields are
private and only reachable that way). Highlights: `background←bg`, `foreground←fg`,
`border←node_border`, `input.border←node_border_strong`, `ring`/`caret←active∨accent`,
`primary←accent`, `secondary`/`popover←node_bg_2`, `muted←node_inset`, list/table roles from
node surfaces plus `accent_bg`/`accent_line`, `selection←accent@0.3`,
danger/success/warning/info←red/green/yellow/blue, `chart.1..5←chart_series`, `base.*←hues`,
`title_bar←bg`, `mode←is_light`. Font family/size: "Monaspace Krypton" 13 px for both UI and mono.
Everything unset falls back along gpui-component's own chain.

## Switching, preview, persistence

`ThemeService { committed, preview }` is a `Global`:
`init(id)`, `preview(id)`, `commit(id) -> changed`, `cancel_preview()`, `effective()`,
`committed()`. `peek_ui::init` calls `ThemeService::init(config.theme, cx)` at startup.

The picker (`peek-ui/src/theme_picker.rs`) reuses the gpui-component `Command` palette in a
dialog: `on_select` previews (arrow keys and hover), `on_confirm` commits and closes,
`on_cancel` / dialog close reverts. It preselects the committed theme. Commit tries
`PeekConfig::save_to_disk(persistence)`, which returns `ConfigError::ReadOnly` until M4.

## Tests

`crates/peek-theme/tests/builtins.rs`: every builtin resolves; `mode` agrees with `is_light`
and with background luminance; required component roles are set and parse as hex; WCAG AA
contrast floors (fg on bg ≥ 4.5, muted on node ≥ 3); selected-row mix moves the right way.
The headless UI test `theme_picker_previews_reverts_and_commits` drives the picker with keys.

## Deferred

Syntax colours into `ThemeConfig.highlight` (gpui-component's `ThemeStyle` has private fields;
build it via JSON when the editor lands), swatch squares in picker rows, bundling the Monaspace
and Chewy font files, per-theme dock icon (`src-tauri/src/dock_icon.rs`, PNGs in
`~/labs/peek/src/assets/`), radial glow approximation for Pine.
