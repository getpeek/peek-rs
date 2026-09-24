# peek-rs documentation

peek-rs is the from-scratch pure-Rust rewrite of Peek, the infinite-canvas database GUI at
`~/labs/peek` (React 19 + React Flow inside Tauri 2). The rewrite runs on
[gpui-kit 0.6.1](https://gpui-kit.com) (gpui-pre 0.3.4 + gpui-component 0.6.1).

Read in this order when picking the project up:

| Doc | What it answers |
|---|---|
| [status.md](status.md) | What works today, what is deferred, how to run and verify |
| [architecture.md](architecture.md) | Crate map, why it is split this way, data flow, key types |
| [canvas.md](canvas.md) | Camera maths, gestures, the custom element, node zoom strategy, frame flow |
| [commands.md](commands.md) | Actions, the command registry, key contexts, keymap file compatibility, palette |
| [themes.md](themes.md) | ThemeSpec tables, the two theme globals, mapping onto gpui-component, picker |
| [testing.md](testing.md) | The three test layers and how to drive the UI headlessly |
| [porting.md](porting.md) | What in the Tauri host is portable and how each module is brought over |
| [release.md](release.md) | Building and installing the app bundle, the signed release pipeline, its secrets |
| [decisions.md](decisions.md) | Decisions made with the user, and the reasoning that is not obvious from code |

`CLAUDE.md` at the repo root holds the rules for working in this codebase (lints, style, gpui-kit
rules). These docs describe the system; CLAUDE.md prescribes how to change it.

The original planning document lives outside the repo at
`~/.claude/plans/this-is-a-migration-parsed-wirth.md`; everything still relevant from it has
been folded into these files.
