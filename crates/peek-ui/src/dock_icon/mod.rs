//! The macOS Dock icon, applied at startup.
//!
//! Under `cargo run` Peek is unbundled, so there is no `.icns` for the Dock to read and it falls
//! back to the generic executable glyph. `-[NSApplication setApplicationIconImage:]` overrides
//! that for the lifetime of the process, which is all an unbundled binary can get. The app
//! bundle `scripts/package.sh` builds renders its `.icns` from this same image.
//!
//! The reference app carries one icon per theme; this is the dark one, used for every theme.

use objc2::AnyThread;
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::{MainThreadMarker, NSData};

/// Sets the Dock icon. Must run on the main thread, which every gpui `App` callback is.
///
/// Gives up quietly if the image is refused: an app without a Dock icon is worth more than
/// an app that refuses to start.
pub(crate) fn apply() {
    let Some(main_thread) = MainThreadMarker::new() else {
        log::warn!("peek: the Dock icon can only be set from the main thread");
        return;
    };

    let data = NSData::with_bytes(include_bytes!("midnight.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        log::warn!("peek: the Dock icon is not a decodable image");
        return;
    };

    // SAFETY: an AppKit setter called on the main thread with an `NSImage` we just built.
    // The application retains the image, so dropping our own reference after the call is fine.
    #[expect(unsafe_code, reason = "gpui exposes no way to set the Dock icon")]
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
    }
}
