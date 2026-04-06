#![allow(dead_code)]
//! Braille spinner for thinking / tool states.

use std::time::{Duration, Instant};

/// Animation frames for long-running work.
#[derive(Debug, Clone)]
pub struct Spinner {
    /// Current frame index in [`Self::FRAMES`].
    pub frame: usize,
    /// Last time the frame advanced.
    pub last_tick: Instant,
    /// Minimum milliseconds between frame advances.
    pub tick_rate_ms: u64,
}

impl Spinner {
    /// Braille sequence used in the TUI.
    pub const FRAMES: &'static [&'static str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

    /// Default 80 ms tick (smooth animation while the agent runs).
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame: 0,
            last_tick: Instant::now(),
            tick_rate_ms: 80,
        }
    }

    /// Advances the frame when `tick_rate_ms` elapsed.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_tick) >= Duration::from_millis(self.tick_rate_ms) {
            self.last_tick = now;
            self.frame = (self.frame + 1) % Self::FRAMES.len();
        }
    }

    /// Current glyph.
    #[must_use]
    pub fn current(&self) -> &'static str {
        Self::FRAMES[self.frame % Self::FRAMES.len()]
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_and_wraps() {
        let mut s = Spinner {
            frame: Spinner::FRAMES.len() - 1,
            last_tick: Instant::now() - Duration::from_millis(200),
            tick_rate_ms: 80,
        };
        s.tick();
        assert_eq!(s.frame, 0);
    }
}
