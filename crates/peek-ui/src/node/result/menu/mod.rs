//! The result table's right-click menus: `CellContextMenu.tsx` and `ResultHeaderMenu.tsx`.
//!
//! The node decides *what is in* the menu; `canvas/context_menu.rs` draws it. The seam between
//! them is a plain list of labelled actions, which is what lets the menu live at canvas level —
//! outside the camera's rem scope, where chrome belongs — without holding a borrow of the node.

pub(crate) mod actions;
mod build;
pub(crate) mod rows;
pub(crate) mod scope;
