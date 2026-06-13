//! Keyboard handling — the BIOS-style key buffer of the original, expressed in
//! crossterm events. Decodes keys into [`super::In`] (INKEY$ semantics), polls
//! with a timeout, and waits for a keystroke (with live theme switching).

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use super::In;
use crate::theme::THEMES;

impl super::Game {
    fn map_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<In> {
        // Ctrl+C / Ctrl+Q quit the whole game (restoring the terminal first).
        if mods.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char('c') | KeyCode::Char('q') = code {
                self.quit();
            }
        }
        match code {
            KeyCode::Up => Some(In::arrow(72)),
            KeyCode::Down => Some(In::arrow(80)),
            KeyCode::Left => Some(In::arrow(75)),
            KeyCode::Right => Some(In::arrow(77)),
            KeyCode::F(9) => Some(In::arrow(67)),  // extra life — shh!
            KeyCode::F(10) => Some(In::arrow(68)), // skip level — shh!
            KeyCode::Esc => Some(In::single(27)),
            KeyCode::Enter => Some(In::single(13)),
            KeyCode::Char(c) => Some(In::single(c as u32)),
            _ => None,
        }
    }
    /// INKEY$ poll with a timeout (ms). Renders the current frame first so the
    /// player always sees the latest state before the game blocks on input.
    pub(super) fn key_or_timeout(&mut self, ms: u64) -> In {
        self.render();
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return In::timeout();
            }
            let slice = (deadline - now).min(Duration::from_millis(50));
            if event::poll(slice).expect("poll") {
                if let Event::Key(k) = event::read().expect("read") {
                    if k.kind != KeyEventKind::Release {
                        if let Some(inp) = self.map_key(k.code, k.modifiers) {
                            return inp;
                        }
                    }
                }
                // resize / other events: keep waiting out the slice
            }
        }
    }
    /// INPUT$(1): wait for a real keystroke. Digits 1-4 hot-swap the theme and
    /// keep waiting, so the player can recolor the CRT at any prompt.
    pub(super) fn wait_key(&mut self) -> In {
        loop {
            let k = self.key_or_timeout(3_600_000);
            if k.len == 0 {
                continue;
            }
            if k.len == 1 && (b'1' as u32..=b'4' as u32).contains(&k.code) {
                self.switch_theme((k.code - b'1' as u32) as usize);
                continue;
            }
            return k;
        }
    }
    pub(super) fn clear_kbd(&mut self) {
        while event::poll(Duration::ZERO).expect("poll") {
            let _ = event::read();
        }
    }
    fn switch_theme(&mut self, idx: usize) {
        if idx < THEMES.len() {
            self.theme = idx;
            self.repaint_all();
            self.save_state();
            self.render();
        }
    }
}
