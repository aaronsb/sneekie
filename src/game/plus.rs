//! Sneekie+ — the survival mode and the boot menu.
//!
//! Classic Sneekie is untouched; everything here runs only when `self.plus` is
//! set. Each level opens with a grace period (normal play). When it expires, the
//! nearest smileys (CP437 `1` / `☺`) turn into hunters (CP437 `2` / `☻`) that
//! step toward the snake's head on every move, and a score multiplier climbs in
//! real time — so the longer you keep eating among them, the richer it gets.
//!
//! The integration points live in [`super::play`], each guarded by `self.plus`.

use std::time::{Duration, Instant};

/// Grace period (seconds) before the first wave wakes, and how it tightens with
/// each cycle — later zombie waves come back faster, down to a floor.
const GRACE_BASE: u64 = 25;
const GRACE_MIN: u64 = 8;
const GRACE_STEP: u64 = 3;
/// How many of the nearest smileys become moving hunters (the rest clear out).
const HUNTERS_MAX: usize = 10;
/// Points it costs to "defeat" a hunter on contact (paid from your score). Bank
/// enough before the swarm wakes and you can ram hunters off the board instead
/// of dying — but only while you can afford it.
const HUNTER_COST: i32 = 75;
/// Fresh faces scattered when a wave is cleared (all hunters defeated).
const WAVE_FACES: i32 = 24;

impl super::Game {
    // ---------------- boot menu ----------------
    /// Two-step boot menu: pick mode, then movement. Returns `(auto, plus, live)`.
    /// Choosing autoplay skips the movement screen. Uses raw key reads so
    /// digits/letters aren't swallowed by the theme switcher.
    pub(super) fn menu(&mut self) -> (bool, bool, bool) {
        // ---- screen 1: mode ----
        self.menu_frame();
        self.locate(9, 34); self.ps("S N E E K I E");
        self.locate(11, 27); self.ps("Choose your game:");
        self.locate(13, 27); self.ps("1  Sneekie (1988)");
        self.locate(14, 27); self.ps("2  Sneekie+  (hunters!)");
        self.locate(15, 27); self.ps("A  Autoplay (watch bot)");
        self.locate(17, 27); self.ps("Press 1, 2 or A");
        let plus = loop {
            let k = self.key_or_timeout(3_600_000);
            if k.len == 1 {
                match (k.code as u8).to_ascii_lowercase() {
                    b'1' => break false,
                    b'2' => break true,
                    b'a' => return (true, self.menu_autoplay(), true), // bot plays
                    _ => {}
                }
            }
        };
        // ---- screen 2: movement ----
        self.menu_frame();
        self.locate(9, 36); self.ps("MOVEMENT");
        self.locate(11, 27); self.ps("How does the snake move?");
        self.locate(13, 27); self.ps("T  Turn-based (per key)");
        self.locate(14, 27); self.ps("L  Live  (always moving)");
        self.locate(17, 27); self.ps("Press T or L");
        let live = loop {
            let k = self.key_or_timeout(3_600_000);
            if k.len == 1 {
                match (k.code as u8).to_ascii_lowercase() {
                    b't' => break false,
                    b'l' => break true,
                    _ => {}
                }
            }
        };
        (false, plus, live)
    }

    /// Autoplay sub-screen: which mode should the bot play? Returns `plus`.
    fn menu_autoplay(&mut self) -> bool {
        self.menu_frame();
        self.locate(9, 35); self.ps("AUTOPLAY");
        self.locate(11, 27); self.ps("Watch the bot play:");
        self.locate(13, 27); self.ps("1  Sneekie (1988)");
        self.locate(14, 27); self.ps("2  Sneekie+  (hunters!)");
        self.locate(17, 27); self.ps("Press 1 or 2");
        loop {
            let k = self.key_or_timeout(3_600_000);
            if k.len == 1 {
                match k.code as u8 {
                    b'1' => return false,
                    b'2' => return true,
                    _ => {}
                }
            }
        }
    }

    /// Clear the screen and draw the empty menu box (rows 8-18, cols 24-56).
    fn menu_frame(&mut self) {
        self.cls();
        self.locate(8, 24); self.pc(201); self.pcn(205, 31); self.pc(187);
        for r in 9..=17 {
            self.locate(r, 24); self.pc(186); self.sp(31); self.pc(186);
        }
        self.locate(18, 24); self.pc(200); self.pcn(205, 31); self.pc(188);
    }

    // ---------------- per-level setup ----------------
    /// Reset the survival clock at the start of a level's move loop.
    pub(super) fn plus_level_init(&mut self) {
        self.danger = false;
        self.mult = 1;
        self.wave = 0;
        self.hunters.clear();
        let g = self.grace_secs();
        self.grace_until = Some(Instant::now() + Duration::from_secs(g));
        self.danger_start = None;
        self.hud_grace(g as i32);
    }

    /// Grace seconds for the upcoming wave: generous at first, shrinking each
    /// cycle down to a floor.
    fn grace_secs(&self) -> u64 {
        // The bot plays Sneekie+ under the same clock as a human — no shortcut.
        GRACE_BASE
            .saturating_sub(GRACE_STEP * self.wave as u64)
            .max(GRACE_MIN)
    }

    // ---------------- per-step: clock, activation, multiplier ----------------
    /// Time-driven update: run the grace countdown, wake the hunters when it
    /// expires, and ramp the multiplier during the danger phase. Called near the
    /// top of every move iteration so wall-clock time always advances.
    pub(super) fn plus_tick(&mut self) {
        // Wave cleared (every hunter defeated)? Take a breather: faces return
        // and a fresh — shorter — clock starts counting toward the next wave.
        if self.danger && self.hunters.is_empty() {
            self.reset_wave();
        }
        let now = Instant::now();
        if !self.danger {
            if let Some(g) = self.grace_until {
                if now >= g {
                    self.activate_hunters(now);
                } else {
                    let secs = (g - now).as_secs() as i32 + 1;
                    self.hud_grace(secs);
                }
            }
        }
        if self.danger {
            if let Some(ds) = self.danger_start {
                let m = 2 + (now - ds).as_secs() as i32;
                if m != self.mult {
                    self.mult = m;
                    self.snd(1046.0, 2.0); // multiplier-up chime
                }
                self.hud_danger();
            }
        }
    }

    /// Turn the nearest smileys into hunters and clear the rest, opening the
    /// field so the threat is the chase, not the clutter.
    fn activate_hunters(&mut self, now: Instant) {
        self.danger = true;
        self.danger_start = Some(now);
        self.mult = 2;
        let head = self.t[self.btel as usize];
        let (hr, hc) = (head / 160, (head % 160) / 2);
        // gather every smiley in the playfield (rows 4-20 → offsets 480..=3198)
        let mut found: Vec<(i32, i32)> = Vec::new();
        let mut off = 480;
        while off <= 3198 {
            if self.peek(off) == 1 {
                let (r, c) = (off / 160, (off % 160) / 2);
                found.push(((hr - r).abs() + (hc - c).abs(), off));
            }
            off += 2;
        }
        found.sort_by_key(|x| x.0);
        self.hunters.clear();
        for (i, (_, o)) in found.into_iter().enumerate() {
            if i < HUNTERS_MAX {
                self.poke(o, 2);
                self.poke(o + 1, 15);
                self.hunters.push(o);
            } else {
                self.poke(o, 32); // far smileys vanish
            }
        }
        // alarm
        self.snd(440.0, 6.0);
        self.snd(330.0, 6.0);
        self.snd(440.0, 6.0);
        self.snd(294.0, 9.0);
        self.hud_danger();
    }

    // ---------------- per-move: hunter movement ----------------
    /// Step every hunter one cell toward the snake's head (along the axis of
    /// greater distance). Reaching the head pays the defeat cost to remove the
    /// hunter if you can afford it, otherwise it's `Err(Death)`. Walls, food,
    /// the snake's body, and other hunters block the step.
    pub(super) fn plus_hunters(&mut self) -> Result<(), super::Death> {
        if !self.danger {
            return Ok(());
        }
        let head = self.t[self.btel as usize];
        let (hr, hc) = (head / 160, (head % 160) / 2);
        let hunters = std::mem::take(&mut self.hunters);
        let mut survivors: Vec<i32> = Vec::with_capacity(hunters.len());
        for off in hunters {
            let (r, c) = (off / 160, (off % 160) / 2);
            let (dr, dc) = (hr - r, hc - c);
            let new = if dr.abs() >= dc.abs() && dr != 0 {
                off + 160 * dr.signum()
            } else if dc != 0 {
                off + 2 * dc.signum()
            } else {
                off
            };
            if new == off {
                survivors.push(off);
                continue;
            }
            match self.peek(new) {
                219 => {
                    // reached the head: pay to defeat it, or die
                    if self.zcore >= HUNTER_COST {
                        self.score(-HUNTER_COST);
                        self.poke(off, 32); // this hunter is gone (not a survivor)
                        self.snd(1200.0, 2.0);
                        self.snd(700.0, 3.0);
                    } else {
                        self.hunters = survivors;
                        return Err(super::Death);
                    }
                }
                32 => {
                    self.poke(off, 32);
                    self.poke(new, 2);
                    self.poke(new + 1, 15);
                    survivors.push(new);
                }
                _ => survivors.push(off), // blocked: hold position this turn
            }
        }
        self.hunters = survivors;
        Ok(())
    }

    /// True if the player can currently afford to defeat a hunter on contact.
    pub(super) fn can_defeat(&self) -> bool {
        self.zcore >= HUNTER_COST
    }
    /// Pay the defeat cost and remove the hunter occupying `off` (used when the
    /// snake rams into a hunter). Caller advances the head into the cell.
    pub(super) fn defeat_hunter_at(&mut self, off: i32) {
        self.score(-HUNTER_COST);
        self.hunters.retain(|&h| h != off);
        self.snd(1200.0, 2.0);
        self.snd(700.0, 3.0);
    }

    /// All hunters defeated: scatter fresh faces and restart the (shorter) clock.
    fn reset_wave(&mut self) {
        self.danger = false;
        self.danger_start = None;
        self.mult = 1;
        self.wave += 1;
        self.scatter_faces(WAVE_FACES);
        let g = self.grace_secs();
        self.grace_until = Some(Instant::now() + Duration::from_secs(g));
        // breather fanfare
        self.snd(523.0, 3.0);
        self.snd(659.0, 3.0);
        self.snd(784.0, 4.0);
        self.hud_grace(g as i32);
    }
    /// Drop `n` fresh smileys onto empty playfield cells.
    fn scatter_faces(&mut self, n: i32) {
        for _ in 0..n {
            self.place(1);
        }
    }

    // ---------------- HUD (spare row 25) ----------------
    fn hud(&mut self, text: &str) {
        self.locate(25, 1);
        self.sp(80);
        self.locate(25, 2);
        let t: String = text.chars().take(77).collect();
        self.ps(&t);
    }
    fn hud_grace(&mut self, secs: i32) {
        let s = format!(
            "SNEEKIE+  wave {} wakes in {:>2}s  bank points! (ram={}pts)  [m] {}",
            self.wave + 1,
            secs.max(0),
            HUNTER_COST,
            if self.muted { "muted" } else { "sound" }
        );
        self.hud(&s);
    }
    fn hud_danger(&mut self) {
        let kills = (self.zcore.max(0) / HUNTER_COST).max(0);
        // While the bot drives, show the live planner readout (cores + total tree
        // nodes searched last tick) so the compute-vs-survival experiment is
        // visible; +/- adjusts the cores.
        let s = if self.auto {
            format!(
                "SNEEKIE+ w{} {}h x{} ram:{} | PLANNER {}c {}nodes  [+/-]",
                self.wave + 1,
                self.hunters.len(),
                self.mult,
                kills,
                self.planner_cores,
                self.plan_nodes,
            )
        } else {
            format!(
                "SNEEKIE+  wave {}  {} hunters  SCORE x{}  ram-kills: {}  [m] {}",
                self.wave + 1,
                self.hunters.len(),
                self.mult,
                kills,
                if self.muted { "muted" } else { "sound" }
            )
        };
        self.hud(&s);
    }

    // ---------------- audio mute toggle ('m') ----------------
    pub(super) fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.save_state();
        if self.danger {
            self.hud_danger();
        } else if let Some(g) = self.grace_until {
            let secs = g.saturating_duration_since(Instant::now()).as_secs() as i32 + 1;
            self.hud_grace(secs);
        }
    }
}
