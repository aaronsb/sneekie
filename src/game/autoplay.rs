//! Autoplay — a self-driving bot.
//!
//! Each step it picks a direction by reading VRAM directly:
//!  1. **BFS** from the head over passable cells (empty / heart / club) to the
//!     nearest food, and take the first step of that shortest path.
//!  2. If no food is reachable, **flood-fill** each candidate neighbor and head
//!     toward the most open space — a cheap survival heuristic that avoids
//!     painting the snake into a corner.
//!
//! Everything else (walls, the snake's own body, smileys, stones, arrows, and
//! Sneekie+ hunters) is treated as impassable, so the bot routes around hazards
//! for free. It's a solid greedy player, not a perfect one — a Hamiltonian
//! cycle would never die but would be slow and dull to watch.

use std::collections::VecDeque;

use crate::theme::theme_index;

/// (scan code, VRAM offset delta) for the four directions.
const DIRS: [(u32, i32); 4] = [(72, -160), (80, 160), (75, -2), (77, 2)];

/// Steps with no score change before the bot gives up and skips the level
/// (~13s at the autoplay tick). Keeps the screensaver cycling rather than
/// safely idling in a maze it can't clear.
const AUTO_STALL: i32 = 180;

/// Move-history ring length (must match `auto_trail` init in mod.rs).
const TRAIL: usize = 64;

/// CP437 codes the snake's own body uses (head + the double-line segments).
/// Distinct from every wall glyph, so these mark *movable* obstacles.
fn is_snake_char(c: u8) -> bool {
    matches!(c, 219 | 186 | 205 | 187 | 188 | 200 | 201)
}

fn reverse(e: u32) -> u32 {
    match e {
        72 => 80,
        80 => 72,
        75 => 77,
        77 => 75,
        _ => 0,
    }
}

fn dir_offset(sc: u32) -> i32 {
    match sc {
        72 => -160,
        80 => 160,
        75 => -2,
        77 => 2,
        _ => 0,
    }
}

impl super::Game {
    /// Cells the bot may travel through: empty and the two foods.
    fn passable(&self, off: i32) -> bool {
        matches!(self.peek(off), 32 | 3 | 5)
    }
    /// Cells worth steering toward: heart (+10) and club (+25).
    fn is_food(&self, off: i32) -> bool {
        matches!(self.peek(off), 3 | 5)
    }

    /// Per-step entry point: pick a move, but bail out of a level the bot can't
    /// make progress on by emitting the F10 "skip level" code once it stalls.
    pub(super) fn auto_choose(&mut self) -> u32 {
        // Move-history ring + cycle detection. How long ago was the head last on
        // this exact cell? A *stable* recurring period is a genuine loop; we only
        // act once that period has matched more than once (more than a single
        // cycle), so a one-off backtrack doesn't trip it.
        let head = self.t[self.btel as usize];
        let ago = self.trail_ago(head);
        self.auto_trail[self.auto_trail_i] = head;
        self.auto_trail_i = (self.auto_trail_i + 1) % TRAIL;
        if ago > 0 {
            if ago as i32 == self.auto_period {
                self.auto_cycles += 1;
            } else {
                self.auto_period = ago as i32;
                self.auto_cycles = 1;
            }
        } else {
            self.auto_period = 0;
            self.auto_cycles = 0;
        }

        if self.zcore != self.auto_last_score {
            self.auto_last_score = self.zcore;
            self.auto_idle = 0;
        } else {
            self.auto_idle += 1;
        }
        if !self.plus {
            // Classic: skip a level the bot can't make progress on so the
            // screensaver keeps moving.
            if self.auto_idle > AUTO_STALL {
                self.auto_idle = 0;
                return 68; // F10: skip
            }
        } else if self.auto_idle > AUTO_STALL * 2 {
            // Sneekie+ gets the human clock (no skip), but it can still wedge
            // itself into a death-spiral the hunters can't reach. Do exactly
            // what the legend tells a stuck human to: press ESC, give up a life,
            // respawn fresh. Costs a life (fair) and self-restarts when they run
            // out.
            self.auto_idle = 0;
            return 27; // ESC
        }
        // The same cycle has matched more than once → break it by steering
        // toward the least-recently-visited neighbor, before the slower
        // score-stall guards have to step in.
        if self.auto_cycles > 1 {
            if let Some(sc) = self.perturb_move(head) {
                return sc;
            }
        }
        self.auto_dir()
    }

    /// Steps since the head was last on `cell` within the ring (0 = not found).
    fn trail_ago(&self, cell: i32) -> usize {
        for j in 1..=TRAIL {
            let idx = (self.auto_trail_i + TRAIL - j) % TRAIL;
            if self.auto_trail[idx] == cell {
                return j;
            }
        }
        0
    }

    /// Pick the passable, non-reverse neighbor the head has visited least in its
    /// recent trail (ties broken by open space) — a nudge toward fresh ground to
    /// break a repeating pattern.
    fn perturb_move(&self, head: i32) -> Option<u32> {
        let rev = reverse(self.e);
        let mut best: Option<u32> = None;
        let mut best_key = (usize::MAX, i32::MIN);
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n < 0 || (n as usize) >= 4000 || !self.passable(n) {
                continue;
            }
            let visits = self.auto_trail.iter().filter(|&&c| c == n).count();
            let key = (visits, -self.flood_count(n)); // fewest visits, then most room
            if best.is_none() || key < best_key {
                best_key = key;
                best = Some(sc);
            }
        }
        best
    }

    /// Choose the next direction (a scan code: 72/80/75/77).
    ///
    /// The hybrid: chase the nearest food greedily, but only commit to a step
    /// that leaves the head still able to reach its own tail (tail-reachability
    /// — the snake can then always escape by following its tail). If the food
    /// step is unsafe, take the safe move that keeps the most room. This makes
    /// it effectively immortal on the static levels; moving enemies (arrows,
    /// hunters) can still corner it, and autoplay just restarts.
    pub(super) fn auto_dir(&self) -> u32 {
        let head = self.t[self.btel as usize];
        let rev = reverse(self.e);

        // 1. Greedy toward food — if that first step keeps the tail reachable.
        if let Some(sc) = self.bfs_food(head, rev) {
            if self.is_safe_move(sc) {
                return sc;
            }
        }
        // 2. Sneekie+: a hunter is right next to us and we can pay? ram it.
        //    Fighting the swarm proactively keeps tight grooves from forming.
        if self.plus && self.can_defeat() {
            for (sc, d) in DIRS {
                if sc == rev {
                    continue;
                }
                let n = head + d;
                if n >= 0 && (n as usize) < 4000 && self.peek(n) == 2 {
                    return sc;
                }
            }
        }
        // 3. Chase the tail: BFS to the tail through open cells and step that
        //    way. Following the body out threads single-cell exits, so the
        //    snake extracts itself from a pocket instead of hugging the
        //    roomiest corner.
        if let Some(sc) = self.bfs_to_tail(head, rev) {
            return sc;
        }
        // 4. Last resort: the most open cell and hope.
        self.most_open(head, rev)
    }

    /// BFS from the head through open cells to the snake's own tail; returns the
    /// first step of that path. The returned step is always into a passable
    /// cell (the tail is only ever *reached from* one).
    fn bfs_to_tail(&self, head: i32, rev: u32) -> Option<u32> {
        let tail = self.t[self.etel as usize];
        let mut seen = vec![false; 4000];
        let mut first = vec![0u32; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                seen[n as usize] = true;
                first[n as usize] = sc;
                q.push_back(n);
            }
        }
        while let Some(c) = q.pop_front() {
            for (_sc, d) in DIRS {
                let n = c + d;
                if n < 0 || (n as usize) >= 4000 || seen[n as usize] {
                    continue;
                }
                if n == tail {
                    return Some(first[c as usize]);
                }
                if self.passable(n) {
                    seen[n as usize] = true;
                    first[n as usize] = first[c as usize];
                    q.push_back(n);
                }
            }
        }
        None
    }

    /// A *static* obstacle: wall / stone / smiley / arrow / hunter — i.e. not
    /// free space, food, or part of the (movable) snake body.
    fn static_blocked(&self, off: i32) -> bool {
        let v = self.peek(off);
        !(matches!(v, 32 | 3 | 5) || is_snake_char(v))
    }

    /// Would stepping `dir` leave the snake able to reach its own tail? If so it
    /// can never trap itself (tail-chasing is always an escape).
    fn is_safe_move(&self, dir: u32) -> bool {
        let head = self.t[self.btel as usize];
        let next = head + dir_offset(dir);
        if next < 0 || (next as usize) >= 4000 || !self.passable(next) {
            return false;
        }
        let eating = self.is_food(next);
        // Body that remains after the move: drop the tail unless we grew.
        let start = if eating { self.etel } else { self.etel + 1 };
        let mut occ = vec![false; 4000];
        for i in start..=self.btel {
            occ[self.t[i as usize] as usize] = true;
        }
        occ[next as usize] = true; // the new head
        let new_tail = self.t[start as usize];
        occ[new_tail as usize] = false; // the tail vacates — it's the goal
        self.reaches(next, new_tail, &occ)
    }

    /// BFS over free cells (static obstacles + the virtual body in `occ`) asking
    /// whether `start` can reach `goal`.
    fn reaches(&self, start: i32, goal: i32, occ: &[bool]) -> bool {
        if start == goal {
            return true;
        }
        let mut seen = vec![false; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[start as usize] = true;
        q.push_back(start);
        while let Some(c) = q.pop_front() {
            for (_sc, d) in DIRS {
                let n = c + d;
                if n < 0 || (n as usize) >= 4000 || seen[n as usize] {
                    continue;
                }
                if n == goal {
                    return true;
                }
                if !self.static_blocked(n) && !occ[n as usize] {
                    seen[n as usize] = true;
                    q.push_back(n);
                }
            }
        }
        false
    }

    /// Per-level reset for the bot: lock to CGA (the only *color* palette — the
    /// screensaver should be in glorious color, not monochrome), and clear the
    /// loop-detection state. Not persisted, so it never clobbers the player's
    /// saved theme.
    pub(super) fn auto_new_level(&mut self) {
        self.theme = theme_index("cga");
        self.repaint_all();
        self.auto_idle = 0;
        self.auto_last_score = self.zcore;
        self.auto_trail.iter_mut().for_each(|c| *c = -1);
        self.auto_trail_i = 0;
        self.auto_period = 0;
        self.auto_cycles = 0;
    }

    /// BFS from the head to the nearest food; returns the first step's scan code.
    fn bfs_food(&self, head: i32, rev: u32) -> Option<u32> {
        let mut seen = vec![false; 4000];
        let mut first = vec![0u32; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        for (sc, d) in DIRS {
            if sc == rev {
                continue; // can't reverse into the neck
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                seen[n as usize] = true;
                first[n as usize] = sc;
                q.push_back(n);
            }
        }
        while let Some(c) = q.pop_front() {
            if self.is_food(c) {
                return Some(first[c as usize]);
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                    seen[n as usize] = true;
                    first[n as usize] = first[c as usize];
                    q.push_back(n);
                }
            }
        }
        None
    }

    /// Pick the non-reverse neighbor opening onto the most reachable space.
    fn most_open(&self, head: i32, rev: u32) -> u32 {
        let mut best = self.e;
        let mut best_space = -1;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && self.passable(n) {
                let space = self.flood_count(n);
                if space > best_space {
                    best_space = space;
                    best = sc;
                }
            }
        }
        best
    }

    /// Count reachable passable cells from `start` (capped, for the fallback).
    fn flood_count(&self, start: i32) -> i32 {
        let mut seen = vec![false; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[start as usize] = true;
        q.push_back(start);
        let mut count = 0;
        while let Some(c) = q.pop_front() {
            count += 1;
            if count > 400 {
                break;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                    seen[n as usize] = true;
                    q.push_back(n);
                }
            }
        }
        count
    }

    /// Bottom-row banner shown while the bot drives a non-plus level.
    pub(super) fn auto_hud(&mut self) {
        self.locate(25, 1);
        self.sp(80);
        self.locate(25, 2);
        let s = format!(
            "AUTOPLAY   level {}   score {}   (the bot is driving — Ctrl+C to quit)",
            self.level, self.zcore
        );
        let t: String = s.chars().take(77).collect();
        self.ps(&t);
    }
}
