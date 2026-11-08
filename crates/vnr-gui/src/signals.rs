// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! SIGINT, SIGTERM and SIGHUP as editor messages.
//!
//! Without this, Ctrl-C in the terminal that started the editor, `kill`,
//! or a closed terminal would end the process with a change still waiting
//! for confirmation, and that unconfirmed change would stay on screen.
//! Instead each signal reaches the editor, which restores the previous
//! configuration before it exits.
//!
//! The first two signals go to the editor, which closes the safe way; a
//! second press of Ctrl-C out of habit changes nothing. A third gets the
//! signal's default action, so a stuck editor can still be stopped.

use std::thread;

use iced::futures::channel::mpsc;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Signals delivered to the editor before the default action takes over.
const DELIVERED: u32 = 2;

/// Starts listening and returns one `()` per signal. The handlers are
/// installed on the listening thread itself, so if that thread or the
/// handlers cannot be set up, the stream is empty and signals keep their
/// default effect.
#[must_use]
pub fn start() -> mpsc::Receiver<()> {
    let (mut sender, receiver) = mpsc::channel(usize::try_from(DELIVERED).unwrap_or(2));
    let _ = thread::Builder::new()
        .name("vernier-signals".into())
        .spawn(move || {
            let Ok(mut signals) = Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
                return;
            };
            let mut seen = 0;
            for signal in signals.forever() {
                seen += 1;
                if seen > DELIVERED || sender.try_send(()).is_err() {
                    let _ = signal_hook::low_level::emulate_default_handler(signal);
                    // The default for these three ends the process; were it
                    // ignored, end it the way a shell reports a signal.
                    std::process::exit(128 + signal);
                }
            }
        });
    receiver
}
