//! The play loop (BASIC 240-1130): level configuration, the per-step movement
//! logic, the death animation, and the boot/restart sequence. This is the
//! program's spine; it leans on [`super::layouts`] for mazes, [`super::enemies`]
//! for hazards, and the machine primitives on [`super::Game`].

use std::time::Duration;

use super::{Death, In, Step};

impl super::Game {
    // ---------------- level config tables (310 + 1010) ----------------
    /// Sets Z/AANTAL/BMIN and builds the maze for this level slot (0..15).
    fn config_level(&mut self, idx: i32) {
        match idx {
            0 => { self.z = 999.0; self.aantal = 75; self.bmin = 10; }
            1 => { self.z = 999.0; self.aantal = 75; self.bmin = 10; self.lay1230(); }
            2 => { self.z = 999.0; self.aantal = 75; self.bmin = 10; self.lay1500(); }
            3 => { self.z = 999.0; self.aantal = 50; self.bmin = 10; self.lay1400(); }
            4 => { self.z = 999.0; self.aantal = 50; self.bmin = 10; self.lay1670(); }
            5 => { self.z = 999.0; self.aantal = 50; self.bmin = 10; self.lay1810(); }
            6 => { self.z = 999.0; self.aantal = 50; self.bmin = 10; self.lay1920(); }
            7 => { self.z = 999.0; self.aantal = 50; self.bmin = 10; self.lay1750(); }
            8 => { self.z = 0.4; self.aantal = 125; self.bmin = 5; }
            9 => { self.z = 0.6; self.aantal = 125; self.bmin = 5; self.lay1230(); }
            10 => { self.z = 0.6; self.aantal = 125; self.bmin = 5; self.lay1500(); }
            11 => { self.z = 0.9; self.aantal = 100; self.bmin = 5; self.lay1400(); }
            12 => { self.z = 0.9; self.aantal = 100; self.bmin = 5; self.lay1670(); }
            13 => { self.z = 1.0; self.aantal = 100; self.bmin = 5; self.lay1810(); }
            14 => { self.z = 1.0; self.aantal = 100; self.bmin = 5; self.lay1920(); }
            _ => { self.z = 1.2; self.aantal = 100; self.bmin = 5; self.lay1750(); }
        }
    }
    fn run_enemy(&mut self, idx: i32) -> Result<(), Death> {
        match idx {
            4 | 7 | 12 | 15 => self.sub2130(),
            5 | 13 => self.sub1830(),
            6 | 14 => self.sub1970(),
            _ => Ok(()),
        }
    }

    // ---------------- death (510-630) ----------------
    fn death_seq(&mut self) {
        for _ in 1..=3 {
            self.snd(2000.0, 3.0);
            self.snd(3000.0, 3.0);
            self.snd(4000.0, 3.0);
            self.snd(3000.0, 3.0);
        }
        self.play_drained();
        while self.etel <= self.btel {
            self.render();
            std::thread::sleep(Duration::from_millis(75));
            let off = self.t[self.etel as usize];
            self.poke(off, 32);
            self.poke(off + 1, 7);
            self.snd(1500.0, 0.1);
            self.etel += 1;
            let pen = -self.bmin;
            self.score(pen);
        }
        self.live -= 1;
        self.hart = 0;
        self.klaver = 0;
        if self.live == 0 {
            self.level = 32;
        } else {
            self.level -= 1;
        }
    }

    /// Glide interval (seconds) for live movement — faster on later levels, and
    /// a touch quicker in Sneekie+ to keep the pressure on.
    fn glide_speed(&self) -> f64 {
        let base = (0.20 - self.level as f64 * 0.003).max(0.08);
        if self.plus {
            base * 0.85
        } else {
            base
        }
    }

    // ---------------- one move-loop iteration (420-1020) ----------------
    fn move_iter(&mut self) -> Result<Step, Death> {
        // Movement style sets the step timeout:
        //  - live: glide on a timer (auto-advance when no key is pressed)
        //  - turn-based: wait for a key; in Sneekie+ still wake every 0.25s so
        //    the grace clock, multiplier, and HUD keep ticking in real time.
        let a = if self.auto {
            // The bot drives: pace the demo (and let Ctrl+C through) then steer.
            let _ = self.key_or_timeout(70);
            let mv = self.auto_choose();
            if mv == 27 {
                In::single(27) // bot pressed ESC — stuck, give up a life
            } else {
                In::arrow(mv)
            }
        } else {
            let z = if self.gliding {
                self.glide_speed()
            } else if self.plus {
                0.25
            } else {
                999.0
            };
            self.key_or_timeout((z * 1000.0) as u64) // 430-460
        };
        if self.bonus > 0 {
            self.bonus -= self.bmin; // 470
        }
        self.locate(23, 73);
        self.pu(6, self.bonus); // 480
        if self.plus {
            self.plus_tick(); // grace countdown / wake hunters / ramp multiplier
        }
        if self.auto && !self.plus {
            self.auto_hud();
        }
        // Turn-based: a timeout only refreshes the clock — the snake (and the
        // hunters, which step with it) hold until you press a direction.
        if a.len == 0 && !self.gliding {
            return Ok(Step::Continue);
        }
        if a.len == 1 {
            // 490
            if a.code == 27 {
                return Err(Death); // 500 -> 510 (ESC)
            }
            if self.plus && (a.code == b'm' as u32 || a.code == b'M' as u32) {
                self.toggle_mute(); // plus: mute toggle, no penalty
                return Ok(Step::Continue);
            }
            // 910: any other key = stall penalty
            self.e = self.f;
            let pen = -self.bmin;
            self.score(pen);
            self.snd(1000.0, 5.0);
            return Ok(Step::Continue);
        }
        if a.len == 2 {
            self.e = a.code; // 640
        }
        let mut aoff = self.t[self.btel as usize]; // 650
        if self.e == 68 {
            return Ok(Step::BreakSkip); // 660: F10
        }
        if self.e == 67 {
            self.live += 1; // 670: F9
            return Ok(Step::BreakSkip);
        }
        if self.e == 80 {
            aoff += 160;
        } else if self.e == 72 {
            aoff -= 160; // 680
        }
        if self.e == 77 {
            aoff += 2;
        } else if self.e == 75 {
            aoff -= 2; // 690
        }
        let d = self.peek(aoff); // 700
        let mut blocked = false;
        match d {
            32 => {
                // 710-730: empty
                let off = self.t[self.etel as usize];
                self.poke(off, 32);
                self.poke(off + 1, 7);
                self.snd(1500.0, 0.1);
                self.etel += 1;
            }
            5 => {
                // 740-760: club +25
                self.place(1);
                self.sub2260();
                self.score(25);
                self.klaver -= 1;
            }
            3 => {
                // 770-790: heart +10
                if self.level > 16 {
                    self.place(5);
                    if self.k1 == 1 {
                        self.klaver += 1;
                    }
                }
                // Sneekie+: stop seeding new faces once the swarm is loose.
                if !(self.plus && self.danger) {
                    self.place(1);
                }
                self.sub2260();
                self.score(10);
                self.hart -= 1;
            }
            10 => {
                // 800-860: push the stone
                let mut ta = aoff;
                if self.e == 80 {
                    ta += 160;
                } else if self.e == 72 {
                    ta -= 160;
                }
                if self.e == 77 {
                    ta += 2;
                } else if self.e == 75 {
                    ta -= 2;
                }
                if self.peek(ta) != 32 {
                    blocked = true; // 840 -> 910
                } else {
                    self.poke(ta, 10);
                    let off = self.t[self.etel as usize];
                    self.poke(off, 32);
                    self.poke(off + 1, 7);
                    self.snd(1500.0, 0.1);
                    self.etel += 1;
                }
            }
            1 => {
                // 870-890: smiley -50
                for i in (1..=50).rev() {
                    self.snd(600.0 + 75.0 * i as f64, 0.35);
                }
                self.score(-50);
                self.place(1);
            }
            24 | 26 | 27 => {
                return Err(Death); // 900: arrow = death
            }
            2 if self.plus => {
                // Sneekie+: ram a hunter — pay the cost to clear it, or die.
                if self.can_defeat() {
                    self.defeat_hunter_at(aoff);
                    let off = self.t[self.etel as usize];
                    self.poke(off, 32);
                    self.poke(off + 1, 7);
                    self.etel += 1; // advance into the cell like an empty step
                } else {
                    return Err(Death);
                }
            }
            _ => {
                blocked = true; // wall / body -> 910
            }
        }
        if blocked {
            // 910
            self.e = self.f;
            let pen = -self.bmin;
            self.score(pen);
            self.snd(1000.0, 5.0);
            return Ok(Step::Continue);
        }
        // 920-970: body corner char from old + new direction
        let (e, f) = (self.e, self.f);
        let bt = self.t[self.btel as usize];
        if (e == 77 && f == 77) || (e == 75 && f == 75) {
            self.poke(bt, 205);
        } else if (e == 80 && f == 80) || (e == 72 && f == 72) {
            self.poke(bt, 186);
        } else if (e == 80 && f == 77) || (e == 75 && f == 72) {
            self.poke(bt, 187);
        } else if (e == 72 && f == 77) || (e == 75 && f == 80) {
            self.poke(bt, 188);
        } else if (e == 80 && f == 75) || (e == 77 && f == 72) {
            self.poke(bt, 201);
        } else if (e == 72 && f == 75) || (e == 77 && f == 80) {
            self.poke(bt, 200);
        }
        self.btel += 1; // 980
        self.t[self.btel as usize] = aoff;
        self.f = self.e;
        self.poke(aoff, 219);
        if self.btel == 15000 {
            return Err(Death); // 990
        }
        // 1000: shimmer along the body
        let mut i = self.btel;
        while i >= self.etel {
            let a1 = self.t[i as usize];
            self.poke(a1 + 1, 15);
            let a2 = self.t[(i - 1) as usize];
            self.poke(a2 + 1, 7);
            i -= 2;
        }
        self.run_enemy((self.level - 1).rem_euclid(16))?; // 1010
        if self.plus {
            self.plus_hunters()?; // hunters chase after the snake has moved
        }
        Ok(Step::Continue)
    }

    // ---------------- 240-1080: FOR LEVEL = 1 TO 32 ----------------
    fn play_levels(&mut self) {
        self.level = 1;
        while self.level <= 32 {
            if self.auto {
                self.auto_new_level(); // lock to CGA color + reset loop detection
            }
            // 250-270: playfield + inner borders
            for i in 1..=17 {
                self.locate(3 + i, 1); self.pc(179); self.sp(78); self.pc(179);
            }
            self.locate(3, 1); self.pc(195); self.pcn(196, 78); self.pc(180);
            self.locate(21, 1); self.pc(195); self.pcn(196, 78); self.pc(180);
            // 280-300: snake start (head row 12, tail row 13, col 41, moving up)
            self.t[1] = 2000;
            self.t[2] = 1840;
            self.btel = 2;
            self.etel = 1;
            let h = self.t[self.btel as usize];
            let tl = self.t[self.etel as usize];
            self.poke(h, 219);
            self.poke(tl, 186);
            self.poke(h + 1, 15);
            self.e = 72;
            self.f = 72;
            self.hart = 0;
            self.klaver = 0;
            self.bonus = 10000;
            self.score(0);
            // 310: level config + walls
            let idx = (self.level - 1).rem_euclid(16);
            self.config_level(idx);
            // 320-330: status values
            self.locate(23, 73); self.pu(6, self.bonus);
            self.locate(23, 61); self.pu(2, self.live);
            self.locate(22, 61); self.pu(2, self.level);
            // 340-360: scatter smileys + hearts. Sneekie+ thins the field to a
            // third so a level is actually clearable under the wave pressure.
            let scatter = if self.plus { (self.aantal / 3).max(12) } else { self.aantal };
            for _ in 1..=scatter {
                self.place(1);
                self.place(3);
                if self.k1 == 1 {
                    self.hart += 1;
                }
            }
            // 370: save area behind popup
            for i in 1..=42 {
                for i3 in 0..=3 {
                    self.s[(i + i3 * 42) as usize] = self.peek(1497 + i + i3 * 160) as i32;
                }
            }
            // 380-400: "Level n" popup
            self.sub2280();
            self.locate(11, 37); self.ps("Level ");
            let lv = self.level;
            self.ps(&format!(" {} ", lv));
            self.locate(12, 34); self.ps("Press any key");
            self.clear_kbd();
            if self.auto {
                self.render();
                std::thread::sleep(Duration::from_millis(700)); // the bot waits for no one
            } else {
                self.wait_key();
            }
            // 410: restore
            for i in 1..=42 {
                for i3 in 0..=3 {
                    let v = self.s[(i + i3 * 42) as usize] as u8;
                    self.poke(1497 + i + i3 * 160, v);
                }
            }

            // 420-1020: the move loop
            if self.plus {
                self.plus_level_init(); // start the grace clock once play begins
            }
            let mut died = false;
            let mut skip = false;
            while self.hart + self.klaver > 0 {
                match self.move_iter() {
                    Ok(Step::Continue) => {}
                    Ok(Step::BreakSkip) => {
                        skip = true;
                        break;
                    }
                    Err(Death) => {
                        self.death_seq();
                        died = true;
                        break;
                    }
                }
            }
            self.mult = 1; // bonus drain (and classic) is never multiplied
            if !died && !skip {
                // 1030-1060: drain bonus into score
                let mut n = 0;
                while self.bonus > 0 {
                    self.score(5);
                    self.bonus -= 5;
                    self.locate(23, 74);
                    self.pu(5, self.bonus);
                    n += 1;
                    if n % 25 == 0 {
                        self.snd(3000.0, 0.1);
                        self.render();
                        std::thread::sleep(Duration::from_millis(8));
                    }
                }
                self.render();
                self.live += 1; // 1070
            }
            self.level += 1;
        }
    }

    // ---------------- 80-210 + 230 + 1090-1130: boot & restart ----------------
    fn draw_chrome(&mut self) {
        self.cls(); // 100
        self.locate(1, 1); self.pc(218); self.pcn(196, 78); self.pc(191); // 110
        self.locate(2, 1); self.pc(179); self.sp(78); self.pc(179); // 120
        self.locate(2, 17); self.ps("**** Sneekie ****         (c) July '88 by HerbySoft");
        self.locate(22, 1); self.pc(179); self.sp(78); self.pc(179); // 140
        self.locate(22, 6); self.ps("10 points      -50 points    Highscore"); // 150
        self.locate(23, 1); self.pc(179); self.sp(78); self.pc(179); // 160
        self.locate(22, 55); self.ps("Level       Score"); // 170
        self.locate(23, 6); self.ps("25 points      Stone         <ESC> when stuck"); // 180
        self.locate(23, 55); self.ps("Lives       Bonus"); // 190
        self.locate(24, 1); self.pc(192); self.pcn(196, 78); self.pc(217); // 200
        self.poke(3396, 1); self.poke(3556, 10); self.poke(3526, 5); self.poke(3366, 3); // 210
        if self.zore > 0 {
            self.locate(22, 46);
            self.pu(6, self.zore);
        }
    }
    pub fn program(&mut self) {
        self.load_state(); // 80 (persisted highscore + theme + mute)
        // Choose mode + movement. With no flags at all, ask via the boot menu;
        // otherwise honor the flags and default the unspecified axis sensibly
        // (Sneekie+ defaults to live, classic defaults to turn-based).
        if self.forced_mode.is_none() && self.forced_live.is_none() && !self.forced_auto {
            let (auto, plus, live) = self.menu();
            self.auto = auto;
            self.plus = plus;
            self.gliding = live;
        } else {
            self.auto = self.forced_auto;
            self.plus = self.forced_mode.unwrap_or(false);
            self.gliding = self.forced_live.unwrap_or(self.auto || self.plus);
        }
        self.draw_chrome();
        loop {
            self.zcore = 0;
            self.live = 3; // 230
            self.play_levels(); // 240-1080
            self.sub2280(); // 1090
            self.locate(11, 37); self.ps("The End");
            self.locate(12, 33); self.ps("Play again (y/n)"); // 1100
            self.clear_kbd(); // 1110
            let again = if self.auto {
                self.render();
                std::thread::sleep(Duration::from_millis(1500));
                true // the bot plays on
            } else {
                loop {
                    let a = self.wait_key(); // 1120
                    if a.len == 1 {
                        let c = a.code as u8 as char;
                        if "YyNnJj".contains(c) {
                            break matches!(c, 'Y' | 'y' | 'J' | 'j');
                        }
                    }
                }
            };
            if again {
                // 1130: full reset, including the static chrome
                self.draw_chrome();
                continue;
            }
            self.cls();
            self.locate(1, 1);
            self.ps("Thanks for playing");
            break;
        }
        self.render();
        self.wait_key(); // any key exits
    }
}
