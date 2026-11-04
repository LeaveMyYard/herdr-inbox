//! The real terminal ([`Screen`] over crossterm and ratatui's diffing
//! terminal): raw mode on the alternate screen, the cursor hidden, keys and
//! resizes in, frames out. Dropping it puts the terminal back.

use std::io::{self, Stdout};
use std::time::Duration;

use ratatui_core::buffer::Buffer;
use ratatui_core::terminal::Terminal;
use ratatui_crossterm::CrosstermBackend;
use ratatui_crossterm::crossterm::event::{self, Event};
use ratatui_crossterm::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui_crossterm::crossterm::{cursor, execute};

use crate::tui::keys;
use crate::tui::screen::{Input, Screen};

/// The process's terminal, taken over.
pub struct Term {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Term {
    /// Takes the terminal over.
    pub fn enter() -> io::Result<Term> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        if let Err(err) = execute!(out, EnterAlternateScreen, cursor::Hide) {
            let _ = disable_raw_mode();
            return Err(err);
        }
        match Terminal::new(CrosstermBackend::new(out)) {
            Ok(terminal) => Ok(Term { terminal }),
            Err(err) => {
                restore();
                Err(err)
            }
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        restore();
    }
}

fn restore() {
    let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

impl Screen for Term {
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Input>> {
        if !event::poll(timeout)? {
            return Ok(None);
        }
        Ok(match event::read() {
            Ok(Event::Key(k)) => keys::input(&k),
            Ok(Event::Resize(..)) => Some(Input::Resize),
            Ok(_) => None,
            Err(_) => Some(Input::Closed), // the terminal went away
        })
    }

    fn draw(&mut self, paint: &mut dyn FnMut(&mut Buffer)) -> io::Result<()> {
        self.terminal.draw(|f| paint(f.buffer_mut()))?;
        Ok(())
    }
}
