//! What the popup's loop needs from a terminal ([`Screen`]) and a clock
//! ([`Clock`]), so the loop runs the same against the real terminal
//! ([`crate::tui::term`]) and against the tests' scripted one.

use std::io;
use std::time::Duration;

use inbox_core::popup::{Key, Millis};
use ratatui_core::buffer::Buffer;

/// What the terminal gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key for the model.
    Key(Key),
    /// Ctrl+Q: close once the sends in flight are recorded (Textual's
    /// `quit` binding, the popup's `action_quit`).
    Quit,
    /// Ctrl+C: Textual's reminder that Ctrl+Q quits.
    HelpQuit,
    /// The terminal changed size: draw again.
    Resize,
    /// The terminal is gone: close.
    Closed,
}

/// The terminal.
pub trait Screen {
    /// The next input, waiting at most `timeout`; `None` if none came.
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Input>>;
    /// Draws a frame: `paint` fills the whole buffer.
    fn draw(&mut self, paint: &mut dyn FnMut(&mut Buffer)) -> io::Result<()>;
}

/// The popup's clocks.
pub trait Clock {
    /// The model's clock (monotonic, any epoch).
    fn now(&self) -> Millis;
    /// Wall-clock time, Unix seconds (ages, busy times).
    fn wall(&self) -> f64;
}

/// The real clocks.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock(std::time::Instant);

impl SystemClock {
    /// The model's clock starts now.
    pub fn start() -> SystemClock {
        SystemClock(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Millis {
        Millis::try_from(self.0.elapsed().as_millis()).unwrap_or(Millis::MAX)
    }

    fn wall(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }
}
