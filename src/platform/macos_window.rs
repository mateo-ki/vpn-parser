//! Native application hiding keeps the window available for Dock activation.
//!
//! Hiding the application is the AppKit operation behind Cmd-H. Unlike hiding
//! only the NSWindow or minimizing it, this lets the Dock restore the existing
//! window without a custom reopen delegate or a separate window thumbnail.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

/// Repair egui-winit's cached minimized flag after a native Dock restore.
///
/// egui-winit 0.36 deliberately does not refresh this flag on macOS. A
/// `Minimized(true)` command therefore survives deminiaturization, causing
/// eframe to run only `App::logic` and leave an unresponsive, unpainted window.
/// This must run in `logic`, since `ui` is skipped in precisely that state.
pub fn repair_minimized_state(ctx: &eframe::egui::Context) {
    if !ctx.input(|input| input.viewport().minimized == Some(true)) {
        return;
    }
    reconcile_minimized_state(ctx, main_window_is_minimized());
}

/// Query AppKit directly on the owning thread, without winit dispatch or locks.
/// The root window has this fixed title; auxiliary dialogs must not decide its
/// minimized state.
fn main_window_is_minimized() -> Option<bool> {
    let main_thread = MainThreadMarker::new()?;
    NSApplication::sharedApplication(main_thread)
        .windows()
        .iter()
        .find(|window| window.title().to_string() == "VProxy")
        .map(|window| window.isMiniaturized())
}

fn reconcile_minimized_state(ctx: &eframe::egui::Context, native_minimized: Option<bool>) {
    if ctx.input(|input| input.viewport().minimized == Some(true))
        && native_minimized == Some(false)
    {
        crate::startup_log("macOS native window restored -> clearing stale minimized state");
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
        ctx.request_repaint();
    }
}

/// Hide the application from its UI thread while the proxy keeps running.
///
/// Return false rather than invoking AppKit from a worker thread. The caller
/// must keep the close request canceled even if hiding cannot be performed.
pub fn hide_application_to_background() -> bool {
    let Some(main_thread) = MainThreadMarker::new() else {
        return false;
    };
    let application = NSApplication::sharedApplication(main_thread);
    application.hide(None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Event, Modifiers, PointerButton, ViewportCommand, ViewportId};

    fn window_input(cached_minimized: bool) -> egui::RawInput {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let viewport = input.viewports.get_mut(&ViewportId::ROOT).unwrap();
        viewport.minimized = Some(cached_minimized);
        viewport.occluded = Some(false);
        viewport.focused = Some(true);
        input
    }

    #[test]
    fn dock_restore_repairs_the_logic_only_state_and_resumes_button_input() {
        let ctx = egui::Context::default();
        let mut input = window_input(true);
        assert_eq!(input.viewport().visible(), Some(false));

        // Dock restores the native window while egui still considers it
        // minimized. No UI pass is available to notice the change.
        let output = ctx.run_logic(&input, |ctx| {
            reconcile_minimized_state(ctx, Some(false));
        });
        let commands = output.viewport_commands.get(&ViewportId::ROOT).unwrap();
        assert!(commands.contains(&ViewportCommand::Minimized(false)));

        // Apply the command the same way egui-winit does, then verify actual
        // widget input in the newly visible viewport.
        for command in commands {
            if let ViewportCommand::Minimized(minimized) = command {
                input
                    .viewports
                    .get_mut(&ViewportId::ROOT)
                    .unwrap()
                    .minimized = Some(*minimized);
            }
        }
        assert_eq!(input.viewport().visible(), Some(true));
        let frame = |input| {
            let mut result = (egui::Rect::NOTHING, false);
            let _ = crate::gui::run_test_ui(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let button = ui.button("Switch page");
                    result = (button.rect, button.clicked());
                });
            });
            result
        };
        let (rect, _) = frame(input.clone());
        let pos = rect.center();
        let mut press = input.clone();
        press.events = vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ];
        frame(press);
        input.events = vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }];
        let (_, clicked) = frame(input);
        assert!(clicked, "restored window must receive button clicks");
    }

    #[test]
    fn a_minimized_or_missing_native_window_is_never_forced_open() {
        for (cached_minimized, native_minimized) in
            [(true, Some(true)), (true, None), (false, Some(false))]
        {
            let ctx = egui::Context::default();
            let output = ctx.run_logic(&window_input(cached_minimized), |ctx| {
                reconcile_minimized_state(ctx, native_minimized);
            });
            assert!(output.viewport_commands.is_empty());
        }
    }
}
