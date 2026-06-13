//! Autoplay — a self-driving bot.
//!
//! On the static maze levels the bot is driven by a bounded forward-search
//! [`planner`]: it snapshots the board into a [`sim`] and beam-searches a
//! multi-move plan toward the heart clusters, committing only to moves that keep
//! its tail reachable, then executes that plan tick-by-tick. Where lookahead
//! can't help — Sneekie+ hunters, or the classic levels with moving arrows — it
//! falls through to a per-tick **greedy** chain instead:
//!  1. **BFS** from the head over passable cells (empty / heart / club) to the
//!     nearest food, taking the first step of that shortest path — if it's safe.
//!  2. (Sneekie+) ram an adjacent hunter if it can pay the cost.
//!  3. **Chase the tail** to thread out of a pocket.
//!  4. Else flood-fill and head toward the **most open space**.
//!
//! Everything else (walls, the snake's own body, smileys, stones, arrows, and
//! Sneekie+ hunters) is treated as impassable, so the bot routes around hazards
//! for free. A short move-history ring also breaks repeating cycles, and stall
//! guards skip (classic) or ESC (Sneekie+) a level it truly can't make progress
//! on, so the screensaver keeps moving.

mod greedy;
mod mcts;
mod planner;
mod sim;

use std::collections::VecDeque;

use crate::theme::theme_index;

/// (scan code, VRAM offset delta) for the four directions.
const DIRS: [(u32, i32); 4] = [(72, -160), (80, 160), (75, -2), (77, 2)];

/// Which planner drives a given tick (see [`super::Game::auto_mode`]).
enum AutoMode {
    Greedy, // reactive chain (moving arrows we can't predict)
    Beam,   // bounded forward search (static board)
    Mcts,   // tree search against the loose swarm (Sneekie+ danger)
}

/// Steps with no score change before the bot gives up and skips the level
/// (~13s at the autoplay tick). Keeps the screensaver cycling rather than
/// safely idling in a maze it can't clear.
const AUTO_STALL: i32 = 180;

/// Move-history ring length (must match `auto_trail` init in game/mod.rs). Long
/// enough to catch the wider loops the bot can fall into between heart clusters.
const TRAIL: usize = 128;

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

        // Progress & wedge detection — two independent give-up signals:
        //  * score stall: no points *gained* for a long stretch (circling a maze
        //    it can't safely clear). Counting only gains — not any change — so a
        //    penalty-bleeding wedge can't keep resetting the counter.
        //  * head wedge: the head literally hasn't moved for several ticks — the
        //    bot is bonking a wall every step. Score-independent, so it fires
        //    fast in either mode no matter how the score is moving.
        if head != self.auto_prev_head {
            self.auto_stuck = 0;
        } else {
            self.auto_stuck += 1;
        }
        self.auto_prev_head = head;
        if self.zcore > self.auto_last_score {
            self.auto_idle = 0;
        } else {
            self.auto_idle += 1;
        }
        if self.zcore != self.auto_last_score {
            // Eating reseeds the field (a new heart + smiley drop at random), so
            // any queued plan is stale — rebuild it from the fresh board.
            self.auto_plan.clear();
        }
        self.auto_last_score = self.zcore;
        // A position with no way to win is a draw: give up the level and move on.
        // Classic uses the F10 skip (no life lost); Sneekie+ presses ESC, which
        // gives up a life and respawns — exactly what the legend tells a stuck
        // human to do, and it self-restarts when lives run out.
        let stall_limit = AUTO_STALL * if self.plus { 2 } else { 1 };
        if self.auto_stuck > 10 || self.auto_idle > stall_limit {
            self.auto_idle = 0;
            self.auto_stuck = 0;
            return if self.plus { 27 } else { 68 };
        }
        // The same cycle has matched more than once → break it by steering
        // toward the least-recently-visited neighbor, before the slower
        // score-stall guards have to step in.
        if self.auto_cycles > 1 {
            if let Some(sc) = self.perturb_move(head) {
                return sc;
            }
        }

        // Pick the brain for this situation (see `auto_mode`).
        match self.auto_mode() {
            // The swarm is loose: search against it. MCTS already models hunter
            // motion, body, walls and ram costs inside its rollouts, so it owns
            // the survival/self-trap trade-off — no `is_safe_move` gate (that
            // ignores hunters and would veto necessary escapes). Fall through to
            // the reactive greedy chain only if the search finds nothing.
            AutoMode::Mcts => {
                self.auto_plan.clear();
                if let Some(mv) = self.plan_mcts() {
                    return mv;
                }
            }
            // Static board: the bounded beam planner does multi-step lookahead
            // toward heart clusters. The planner ranks tail-safety only as a
            // (large) bonus, so every committed move still passes the proven
            // one-ply `is_safe_move` gate — the planner supplies the routing, the
            // gate supplies the never-self-trap guarantee. If its best move isn't
            // safe, drop the plan and defer to greedy.
            AutoMode::Beam => {
                let stale = self
                    .auto_plan
                    .front()
                    .is_none_or(|&mv| !self.plan_safe_move(mv));
                if stale {
                    self.auto_plan = self.plan_path().unwrap_or_default();
                }
                if let Some(&mv) = self.auto_plan.front() {
                    if self.plan_safe_move(mv) {
                        self.auto_plan.pop_front();
                        return mv;
                    }
                    self.auto_plan.clear();
                }
            }
            // Moving arrows we can't yet predict: stay fully reactive.
            AutoMode::Greedy => self.auto_plan.clear(),
        }
        self.auto_dir()
    }

    /// Which planner drives this tick:
    /// - **Mcts** — Sneekie+ with the swarm loose (the deterministic, searchable
    ///   adversary). Eating doesn't reseed during danger, so rollouts are exact.
    /// - **Greedy** — level slots that run moving arrows; their motion isn't
    ///   modeled yet, so neither lookahead form is trustworthy.
    /// - **Beam** — everything else: classic static levels and the Sneekie+
    ///   grace phase (no hunters yet — the same static routing problem).
    fn auto_mode(&self) -> AutoMode {
        let arrow = matches!((self.level - 1).rem_euclid(16), 4 | 5 | 6 | 7 | 12 | 13 | 14 | 15);
        if arrow {
            AutoMode::Greedy
        } else if self.plus && self.danger {
            AutoMode::Mcts
        } else {
            AutoMode::Beam
        }
    }

    /// A *static* obstacle: wall / stone / smiley / arrow / hunter — i.e. not
    /// free space, food, or part of the (movable) snake body.
    fn static_blocked(&self, off: i32) -> bool {
        let v = self.peek(off);
        !(matches!(v, 32 | 3 | 5) || is_snake_char(v))
    }

    /// A *hard* obstacle: everything `static_blocked` flags except a smiley.
    /// Smileys are passable at a -50 cost (eat through one when it's the only way
    /// forward), so they must not wall off a region for the planner.
    fn hard_blocked(&self, off: i32) -> bool {
        self.static_blocked(off) && self.peek(off) != 1
    }

    /// Would stepping `dir` leave the snake able to reach its own tail? If so it
    /// can never trap itself (tail-chasing is always an escape). Used to gate the
    /// greedy fallback, which only ever steps onto empty/food (`passable`).
    fn is_safe_move(&self, dir: u32) -> bool {
        let head = self.t[self.btel as usize];
        let next = head + dir_offset(dir);
        if next < 0 || (next as usize) >= 4000 || !self.passable(next) {
            return false;
        }
        self.tail_reachable_after(next, self.is_food(next), false)
    }

    /// Tail-safety gate for a *planner* move, which may deliberately step onto a
    /// smiley. Allows smiley/heart/club/empty steps and treats smileys as
    /// passable when checking that the tail stays reachable.
    fn plan_safe_move(&self, dir: u32) -> bool {
        let head = self.t[self.btel as usize];
        let next = head + dir_offset(dir);
        if next < 0 || (next as usize) >= 4000 || self.hard_blocked(next) {
            return false;
        }
        // Heart, club and smiley all grow (the tail stays); empty vacates it.
        let grows = self.is_food(next) || self.peek(next) == 1;
        self.tail_reachable_after(next, grows, true)
    }

    /// Shared core of the two gates: after moving the head to `next` (growing if
    /// `grows`), can it still reach the vacated tail? `soft_smiley` lets the
    /// reachability flood pass through smileys.
    fn tail_reachable_after(&self, next: i32, grows: bool, soft_smiley: bool) -> bool {
        // Body that remains after the move: drop the tail unless we grew.
        let start = if grows { self.etel } else { self.etel + 1 };
        let mut occ = vec![false; 4000];
        for i in start..=self.btel {
            occ[self.t[i as usize] as usize] = true;
        }
        occ[next as usize] = true; // the new head
        let new_tail = self.t[start as usize];
        occ[new_tail as usize] = false; // the tail vacates — it's the goal
        self.reaches(next, new_tail, &occ, soft_smiley)
    }

    /// BFS over free cells (obstacles + the virtual body in `occ`) asking whether
    /// `start` can reach `goal`. With `soft_smiley`, smileys count as passable.
    fn reaches(&self, start: i32, goal: i32, occ: &[bool], soft_smiley: bool) -> bool {
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
                let blocked = if soft_smiley {
                    self.hard_blocked(n)
                } else {
                    self.static_blocked(n)
                };
                if !blocked && !occ[n as usize] {
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
        self.auto_stuck = 0;
        self.auto_prev_head = -1;
        self.auto_plan.clear();
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
