//! The game itself — a faithful port of SNEEKIE.BAS.
//!
//! [`Game`] is the 1988 program; it owns the 80x25 "VRAM" and pokes characters
//! into it exactly as the GW-BASIC original POKE'd video memory at B000/B800.
//! `offset = (row-1)*160 + (col-1)*2`; even bytes are CP437 codes, odd bytes are
//! attributes (7 = normal, 15 = bright). Because that layout is preserved
//! byte-for-byte, the maze layouts and enemy routines port straight from the
//! BASIC — the comments carry the original line numbers.
//!
//! This module is the "machine": VRAM, the CRT renderer, the keyboard, and the
//! GW-BASIC output primitives. The game's content is split across siblings:
//! - [`layouts`] — the eight maze builders (`lay*`)
//! - [`enemies`] — the arrow/gap hazards (`sub*`)
//! - [`play`]    — the level loop, movement, death, and boot sequence
//!
//! Audio (the GW-BASIC `SOUND` calls) is a deliberate no-op: terminal audio is
//! a poor substitute for a square-wave PC speaker, and the draw is the CP437
//! visuals. The `snd()` hooks mark where the tones fired.

#[cfg(feature = "audio")]
mod audio;
mod autoplay;
mod enemies;
mod input;
mod layouts;
mod play;
mod plus;

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, SetBackgroundColor, SetForegroundColor},
    terminal::{
        disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

use crate::cp437::CP437;
use crate::theme::{cga_color, cga_rgb, theme_index, THEMES};

const VW: usize = 80; // text columns
const VH: usize = 25; // text rows

/// One decoded keystroke, modeled on the original's INKEY$ semantics:
/// `len == 0` → timeout, `len == 1` → ASCII key (Esc/Enter/letter),
/// `len == 2` → extended scan code (arrows, function keys).
#[derive(Clone, Copy)]
struct In {
    len: u8,
    code: u32,
}
impl In {
    fn timeout() -> Self { In { len: 0, code: 0 } }
    fn single(c: u32) -> Self { In { len: 1, code: c } }
    fn arrow(s: u32) -> Self { In { len: 2, code: s } }
}

/// Death signal — returned by collisions and propagated with `?`,
/// matching the BASIC `RETURN 510`.
struct Death;

/// Result of one move-loop iteration.
enum Step {
    Continue,
    BreakSkip, // F9/F10 left the level
}

pub struct Game {
    out: Stdout,
    // ---- VIDEO: 80x25 text VRAM, identical to B000/B800 layout ----
    vram: Vec<u8>,    // 4000 bytes
    dirty: Vec<bool>, // 2000 cells
    theme: usize,
    forced_theme: Option<usize>, // CLI override beats persisted theme
    rng: u64,
    // ---- cursor (GW-BASIC LOCATE) ----
    cur_r: i32,
    cur_c: i32,
    // ---- game state (names as in the BASIC) ----
    t: Vec<i32>,      // T(15000): snake cell offsets
    s: Vec<i32>,      // S(168): popup backup
    b: [i32; 11],     // B(10): gate positions
    d: Vec<[i32; 4]>, // D(80,3): arrows
    zore: i32,        // highscore
    zcore: i32,       // score
    live: i32,
    level: i32,
    btel: i32, // head index into T
    etel: i32, // tail index into T
    e: u32,    // current direction (scan code)
    f: u32,    // previous direction
    hart: i32, // hearts remaining
    klaver: i32, // clubs remaining
    bonus: i32,
    aantal: i32, // items to scatter
    bmin: i32,   // per-move penalty unit
    z: f64,      // step timeout in seconds
    k1: i32,     // last place() succeeded
    mult: i32,   // score multiplier (1 in classic; ramps in Sneekie+ danger phase)
    // ---- mode + movement ----
    forced_mode: Option<bool>,       // CLI: Some(true)=+, Some(false)=classic, None=ask
    forced_live: Option<bool>,       // CLI: Some(true)=live, Some(false)=turn-based, None=ask
    forced_auto: bool,               // CLI: --auto starts the self-driving bot
    plus: bool,                      // survival mode (hunters) active this run
    gliding: bool,                   // movement: true=always gliding, false=move-per-keypress
    auto: bool,                      // the bot is driving
    auto_idle: i32,                  // autoplay: steps since the score last moved
    auto_last_score: i32,            // autoplay: last score seen (stall detection)
    auto_trail: Vec<i32>,            // autoplay: ring of recent head cells (len 64; loop detection)
    auto_trail_i: usize,             // autoplay: ring write index
    auto_period: i32,                // autoplay: last detected cycle length
    auto_cycles: i32,                // autoplay: consecutive same-period matches
    auto_plan: std::collections::VecDeque<u32>, // autoplay: queued planner moves (scan codes)
    muted: bool,                     // audio muted (Sneekie+ only)
    grace_until: Option<Instant>,    // when the current level's grace period ends
    danger: bool,                    // hunters are loose
    danger_start: Option<Instant>,   // when the danger phase began (drives the multiplier)
    hunters: Vec<i32>,               // VRAM offsets of the active hunters (CP437 2)
    wave: i32,                       // hunter cycles elapsed this level (grace shrinks with it)
    #[cfg(feature = "audio")]
    audio: Option<audio::Audio>,
    // ---- persistence ----
    save_path: Option<std::path::PathBuf>,
}

impl Drop for Game {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.out, LeaveAlternateScreen, Show);
    }
}

impl Game {
    pub fn new(
        forced_theme: Option<usize>,
        forced_mode: Option<bool>,
        forced_live: Option<bool>,
        forced_auto: bool,
        save_path: Option<std::path::PathBuf>,
    ) -> Self {
        Game {
            out: io::stdout(),
            vram: vec![0u8; 4000],
            dirty: vec![false; 2000],
            theme: forced_theme.unwrap_or_else(|| theme_index("cga")),
            forced_theme,
            rng: 0,
            cur_r: 1,
            cur_c: 1,
            t: vec![0i32; 15001],
            s: vec![0i32; 169],
            b: [0; 11],
            d: vec![[0i32; 4]; 81],
            zore: 0,
            zcore: 0,
            live: 0,
            level: 0,
            btel: 0,
            etel: 0,
            e: 72,
            f: 72,
            hart: 0,
            klaver: 0,
            bonus: 0,
            aantal: 0,
            bmin: 0,
            z: 999.0,
            k1: 0,
            mult: 1,
            forced_mode,
            forced_live,
            forced_auto,
            plus: false,
            gliding: false,
            auto: false,
            auto_idle: 0,
            auto_last_score: 0,
            auto_trail: vec![-1; 64], // must match TRAIL in autoplay.rs
            auto_trail_i: 0,
            auto_period: 0,
            auto_cycles: 0,
            auto_plan: std::collections::VecDeque::new(),
            muted: false,
            grace_until: None,
            danger: false,
            danger_start: None,
            hunters: Vec::new(),
            wave: 0,
            #[cfg(feature = "audio")]
            audio: audio::Audio::new(),
            save_path,
        }
    }

    // ---------------- terminal lifecycle ----------------
    pub fn init_terminal(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(
            self.out,
            EnterAlternateScreen,
            Hide,
            SetBackgroundColor(Color::Rgb { r: 0, g: 0, b: 0 }),
            Clear(ClearType::All)
        )
    }
    /// Block until the terminal is at least 80x25, prompting the player to
    /// enlarge the window. Ctrl+C/Ctrl+Q quits.
    pub fn ensure_size(&mut self) {
        loop {
            let (c, r) = size().unwrap_or((0, 0));
            if c as usize >= VW && r as usize >= VH {
                return;
            }
            let _ = execute!(self.out, Clear(ClearType::All), MoveTo(0, 0));
            let msg = format!(
                "Sneekie needs an {}x{} terminal.  Current: {}x{}.\r\n\r\nEnlarge the window…  (Ctrl+C to quit)",
                VW, VH, c, r
            );
            let _ = execute!(
                self.out,
                SetForegroundColor(Color::Rgb { r: 255, g: 196, b: 56 }),
                Print(msg)
            );
            let _ = self.out.flush();
            if event::poll(Duration::from_millis(250)).unwrap_or(false) {
                if let Ok(Event::Key(k)) = event::read() {
                    if k.modifiers.contains(KeyModifiers::CONTROL) {
                        if let KeyCode::Char('c') | KeyCode::Char('q') = k.code {
                            self.quit();
                        }
                    }
                }
            }
        }
    }
    fn quit(&mut self) -> ! {
        let _ = disable_raw_mode();
        let _ = execute!(self.out, LeaveAlternateScreen, Show);
        std::process::exit(0);
    }

    // ---------------- VRAM access ----------------
    fn poke(&mut self, off: i32, v: u8) {
        if off >= 0 && (off as usize) < 4000 {
            let o = off as usize;
            if self.vram[o] != v {
                self.vram[o] = v;
                self.dirty[o >> 1] = true;
            }
        }
    }
    fn peek(&self, off: i32) -> u8 {
        if off >= 0 && (off as usize) < 4000 {
            self.vram[off as usize]
        } else {
            0
        }
    }

    // ---------------- GW-BASIC output helpers ----------------
    fn locate(&mut self, r: i32, c: i32) {
        self.cur_r = r;
        self.cur_c = c;
    }
    fn wch(&mut self, code: u8) {
        let off = (self.cur_r - 1) * 160 + (self.cur_c - 1) * 2;
        self.poke(off, code);
        self.poke(off + 1, 7);
        self.cur_c += 1;
    }
    fn ps(&mut self, s: &str) {
        for c in s.chars() {
            let cp = c as u32;
            self.wch(if cp < 128 { cp as u8 } else { 63 });
        }
    }
    fn pc(&mut self, code: u8) {
        self.wch(code);
    }
    fn pcn(&mut self, code: u8, n: i32) {
        for _ in 0..n {
            self.wch(code);
        }
    }
    fn sp(&mut self, n: i32) {
        self.pcn(32, n);
    }
    /// PRINT USING "#..#": right-justify an integer in width `w`.
    fn pu(&mut self, w: usize, n: i32) {
        let s = n.to_string();
        let s = if s.len() < w { format!("{:>width$}", s, width = w) } else { s };
        self.ps(&s);
    }
    fn cls(&mut self) {
        for i in (0..4000).step_by(2) {
            self.vram[i] = 32;
            self.vram[i + 1] = 7;
        }
        self.repaint_all();
        self.locate(1, 1);
    }
    fn repaint_all(&mut self) {
        for d in self.dirty.iter_mut() {
            *d = true;
        }
    }

    // ---------------- rendering ----------------
    fn cell_color(&self, ch: u8, at: u8) -> (u8, u8, u8) {
        let th = &THEMES[self.theme];
        if th.cga {
            cga_rgb(cga_color(ch, at))
        } else if at & 8 != 0 {
            th.bright
        } else {
            th.dim
        }
    }
    fn render(&mut self) {
        for i in 0..2000 {
            if self.dirty[i] {
                self.dirty[i] = false;
                let ch = self.vram[i * 2];
                let at = self.vram[i * 2 + 1];
                let (r, g, b) = self.cell_color(ch, at);
                let row = (i / VW) as u16;
                let col = (i % VW) as u16;
                queue!(
                    self.out,
                    MoveTo(col, row),
                    SetForegroundColor(Color::Rgb { r, g, b }),
                    Print(CP437[ch as usize])
                )
                .expect("render");
            }
        }
        self.out.flush().expect("flush");
    }

    // ---------------- persistence ----------------
    fn load_state(&mut self) {
        if let Some(p) = &self.save_path {
            if let Ok(txt) = std::fs::read_to_string(p) {
                for line in txt.lines() {
                    if let Some(v) = line.strip_prefix("highscore=") {
                        if let Ok(n) = v.trim().parse::<i32>() {
                            self.zore = n;
                        }
                    } else if let Some(v) = line.strip_prefix("theme=") {
                        if self.forced_theme.is_none() {
                            self.theme = theme_index(v.trim());
                        }
                    } else if let Some(v) = line.strip_prefix("muted=") {
                        self.muted = v.trim() == "1";
                    }
                }
            }
        }
    }
    fn save_state(&self) {
        if let Some(p) = &self.save_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let body = format!(
                "highscore={}\ntheme={}\nmuted={}\n",
                self.zore,
                THEMES[self.theme].name,
                if self.muted { 1 } else { 0 }
            );
            let _ = std::fs::write(p, body);
        }
    }

    // ---------------- sound ----------------
    // GW-BASIC `SOUND freq, ticks` — ticks are 1/18.2s PC-clock units. With the
    // `audio` feature this drives a square-wave synth; otherwise it's a no-op.
    #[cfg(feature = "audio")]
    fn snd(&self, freq: f64, ticks: f64) {
        if let Some(a) = self.audio.as_ref() {
            a.beep(freq, ticks / 18.2, self.muted);
        }
    }
    #[cfg(not(feature = "audio"))]
    fn snd(&self, _freq: f64, _ticks: f64) {}
    fn play_drained(&self) {}
    /// 2260: eat-arpeggio.
    fn sub2260(&self) {
        self.snd(2500.0, 0.1);
        self.snd(3500.0, 0.1);
        self.snd(5000.0, 0.1);
    }

    // ---------------- tiny PRNG (replaces Math.random) ----------------
    fn rnd(&mut self) -> f64 {
        // xorshift64* seeded once from the clock, deterministic thereafter.
        if self.rng == 0 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15);
            self.rng = nanos | 1;
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64
    }

    // ---------------- placement / scoring ----------------
    /// 1150: drop item L on a random empty cell (rows 4-20).
    fn place(&mut self, l: u8) {
        self.k1 = 0;
        let mut k = (self.rnd() * 2720.0 + 480.0).trunc() as i32;
        if k % 2 == 1 {
            k += 1;
        }
        if self.peek(k) == 32 {
            self.poke(k, l);
            self.k1 = 1;
        }
    }
    /// 1190: score OP points, track the highscore. In Sneekie+'s danger phase,
    /// positive points are scaled by the survival multiplier; penalties are not.
    /// `mult` is always 1 in classic mode, so this is a no-op there.
    fn score(&mut self, op: i32) {
        let op = if op > 0 { op * self.mult } else { op };
        self.zcore += op;
        self.locate(22, 73);
        self.pu(6, self.zcore);
        if self.zcore > self.zore {
            self.zore = self.zcore;
            self.locate(22, 46);
            self.pu(6, self.zore);
            self.save_state();
        }
    }
    /// 1480.
    fn stone(&mut self, x: i32, y: i32) {
        self.poke((y - 1) * 160 + (x - 1) * 2, 10);
    }
    /// 2280: popup box, rows 10-13, cols 30-50.
    fn sub2280(&mut self) {
        self.locate(10, 30); self.pc(201); self.pcn(205, 19); self.pc(187);
        self.locate(11, 30); self.pc(186); self.sp(19); self.pc(186);
        self.locate(12, 30); self.pc(186); self.sp(19); self.pc(186);
        self.locate(13, 30); self.pc(200); self.pcn(205, 19); self.pc(188);
    }
}
