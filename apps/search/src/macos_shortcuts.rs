// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    cell::Cell,
    ptr::{self, NonNull},
    rc::Rc,
};

use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags as Flags};
use serde::Serialize;
use tao::{event_loop::EventLoopProxy, platform::macos::WindowExtMacOS, window::Window};

use crate::AppEvent;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Shortcut {
    key: &'static str,
    meta_key: bool,
    ctrl_key: bool,
    shift_key: bool,
}

impl Shortcut {
    pub(crate) fn focuses_address(&self) -> bool {
        self.key == "l"
    }
}

fn shortcut(key: &str, flags: Flags, repeating: bool) -> Option<Shortcut> {
    let relevant = flags & (Flags::Command | Flags::Control | Flags::Option | Flags::Shift);
    let shift = relevant.contains(Flags::Shift);
    let primary = relevant - Flags::Shift;
    let key = match (primary, key) {
        (Flags::Control, "\t" | "\u{19}") => "Tab",
        (Flags::Command, "t" | "T") if !repeating => "t",
        (Flags::Command, "w" | "W") if !shift && !repeating => "w",
        (Flags::Command, "l" | "L") if !shift => "l",
        (Flags::Command, "r" | "R") if !shift => "r",
        (Flags::Command, "[") if !shift => "[",
        (Flags::Command, "]") if !shift => "]",
        (Flags::Command, digit) if !shift => {
            const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
            *DIGITS.iter().find(|&&candidate| candidate == digit)?
        }
        _ => return None,
    };
    Some(Shortcut {
        key,
        meta_key: primary == Flags::Command,
        ctrl_key: primary == Flags::Control,
        shift_key: shift,
    })
}

pub(crate) struct Monitor(Retained<AnyObject>);

impl Monitor {
    pub(crate) fn install(
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
        enabled: Rc<Cell<bool>>,
    ) -> Option<Self> {
        let window = window.ns_window();
        let block = RcBlock::new(move |event: NonNull<NSEvent>| {
            let original = event.as_ptr();
            if !enabled.get() {
                return original;
            }
            // AppKit supplies a live NSEvent on the main thread for this callback.
            let event = unsafe { event.as_ref() };
            let Some(main) = MainThreadMarker::new() else {
                return original;
            };
            let Some(target) = event.window(main) else {
                return original;
            };
            if Retained::as_ptr(&target).cast::<std::ffi::c_void>() != window.cast_const() {
                return original;
            }
            let flags = event.modifierFlags();
            if !flags.intersects(Flags::Control | Flags::Command) {
                return original;
            }
            let Some(key) = event.charactersIgnoringModifiers() else {
                return original;
            };
            let Some(shortcut) = shortcut(&key.to_string(), flags, event.isARepeat()) else {
                return original;
            };
            if proxy.send_event(AppEvent::NativeShortcut(shortcut)).is_ok() {
                ptr::null_mut()
            } else {
                original
            }
        });
        // The block returns its input pointer or null and never retains the borrowed event.
        unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        }
        .map(Self)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // The token is the live monitor returned by AppKit.
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_browser_shortcuts_without_intercepting_typing() {
        assert_eq!(shortcut("t", Flags::Command, false).unwrap().key, "t");
        assert_eq!(
            shortcut("T", Flags::Command | Flags::Shift, false)
                .unwrap()
                .key,
            "t"
        );
        assert!(
            shortcut("\t", Flags::Control | Flags::Shift, false)
                .unwrap()
                .shift_key
        );
        assert_eq!(shortcut("9", Flags::Command, false).unwrap().key, "9");
        assert!(shortcut("t", Flags::Command, true).is_none());
        assert!(shortcut("w", Flags::Command, true).is_none());
        assert!(shortcut("t", Flags::Command | Flags::Option, false).is_none());
        assert!(shortcut("c", Flags::Command, false).is_none());
        assert!(shortcut("l", Flags::empty(), false).is_none());
    }
}
