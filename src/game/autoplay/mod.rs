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
mod navigate;
mod planner;
mod sim;

use std::collections::VecDeque;

use crate::theme::theme_index;

/// (scan code, VRAM offset delta) for the four directions.
const DIRS: [(u32, i32); 4] = [(72, -160), (80, 160), (75, -2), (77, 2)];

/// Which planner drives a given tick (see [`super::Game::auto_mode`]). The greedy
/// chain (`auto_dir`) is the shared fallback beneath all of these.
enum AutoMode {
    Beam,       // bounded forward search (static board)
    Mcts,       // tree search against the loose swarm (Sneekie+ danger)
    Predictive, // time-aware routing through moving arrows / wall gaps
}

/// Steps with no score change before the bot gives up and skips the level
/// (~13s at the autoplay tick). Keeps the screensaver cycling rather than
/// safely idling in a maze it can't clear.
const AUTO_STALL: i32 = 180;

/// Move-history ring length (must match `auto_trail` init in game/mod.rs). Long
/// enough to catch the wider loops the bot can fall into between heart clusters.
const TRAIL: usize = 128;

/// Minimum head maneuvering pocket the safety gate insists on when reachable
/// space is below the snake's length. Small, so the bot can still thread tight
/// endgames — just not into a near-dead one- or two-cell pocket.
const ROOM_FLOOR: i32 = 4;

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
            // Moving arrows / wall gaps: forecast them and route a timed path.
            // Replans every tick (hazards move); greedy backs it up if the search
            // finds nothing safe.
            AutoMode::Predictive => {
                self.auto_plan.clear();
                if let Some(mv) = self.navigate() {
                    return mv;
                }
            }
        }
        self.auto_dir()
    }

    /// Live keyboard during autoplay: `+`/`-` adjust how many cores the parallel
    /// planner may use (clamped to the hardware-thread ceiling), for watching how
    /// much more compute actually helps the bot survive the swarm.
    pub(super) fn auto_handle_key(&mut self, code: u32) {
        match code as u8 {
            b'+' | b'=' => self.planner_cores = (self.planner_cores + 1).min(self.planner_max),
            b'-' | b'_' => self.planner_cores = self.planner_cores.saturating_sub(1).max(1),
            _ => {}
        }
    }

    /// Which planner drives this tick:
    /// - **Predictive** — level slots with moving arrows or crawling wall gaps:
    ///   their motion is deterministic, so forecast it and route a timed path.
    /// - **Mcts** — Sneekie+ with the swarm loose (the deterministic, searchable
    ///   adversary). Eating doesn't reseed during danger, so rollouts are exact.
    /// - **Beam** — everything else: classic static levels and the Sneekie+
    ///   grace phase (no hunters yet — the same static routing problem).
    fn auto_mode(&self) -> AutoMode {
        let moving = matches!((self.level - 1).rem_euclid(16), 4 | 5 | 6 | 7 | 12 | 13 | 14 | 15);
        if moving {
            AutoMode::Predictive
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

    /// A *hard* obstacle: a true wall/arrow. Smileys (passable at a -50 cost) and
    /// pushable stones (shovable when the far side is clear) are deliberately left
    /// out so they can't wall off a region for the planner — the `Sim` models
    /// what actually happens when the snake enters them.
    fn hard_blocked(&self, off: i32) -> bool {
        self.static_blocked(off) && !matches!(self.peek(off), 1 | 10)
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
        self.move_keeps_room(next, self.is_food(next), false)
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
        // A stone is only a legal move if it can be pushed: the cell one further
        // in the same direction must be empty. A push doesn't grow the snake.
        if self.peek(next) == 10 {
            let beyond = next + dir_offset(dir);
            if !(beyond >= 0 && (beyond as usize) < 4000 && self.peek(beyond) == 32) {
                return false;
            }
            return self.move_keeps_room(next, false, true);
        }
        // Heart, club and smiley all grow (the tail stays); empty vacates it.
        let grows = self.is_food(next) || self.peek(next) == 1;
        self.move_keeps_room(next, grows, true)
    }

    /// Shared core of the two gates — a one-ply backstop over the tail-aware beam
    /// planner, so keep it permissive. After moving the head to `next` (growing
    /// if `grows`), a move is safe if the free space reachable from the new head
    /// (tail vacated, smileys optionally passable) is at least the snake's length
    /// — ample room — OR it's a tighter spot that still has a real maneuvering
    /// pocket (`ROOM_FLOOR`+ cells) AND keeps the tail reachable, so the snake
    /// can follow itself out. The floor is the key: it stops a one-wide dead
    /// channel from passing as "tail-reachable" (the seal-yourself bug) without
    /// forbidding the legitimately tight moves needed to grab the last hearts.
    fn move_keeps_room(&self, next: i32, grows: bool, soft_smiley: bool) -> bool {
        let start = if grows { self.etel } else { self.etel + 1 };
        let mut occ = vec![false; 4000];
        for i in start..=self.btel {
            occ[self.t[i as usize] as usize] = true;
        }
        occ[next as usize] = true; // the new head
        let tail = self.t[start as usize];
        occ[tail as usize] = false; // the tail vacates
        let base = self.btel - self.etel + 1;
        let len = if grows { base + 1 } else { base };
        let (room, reached_tail) = self.head_room(next, &occ, soft_smiley, tail, len);
        room >= len || (room >= ROOM_FLOOR && reached_tail)
    }

    /// Flood free cells reachable from `head` over non-obstacle, non-body cells,
    /// counting up to `cap` and noting whether the vacated `tail` was reached.
    /// When the count stays below `cap` the flood is exhaustive, so `reached_tail`
    /// is exact. With `soft_smiley`, smileys count as free (passable at a cost).
    fn head_room(&self, head: i32, occ: &[bool], soft_smiley: bool, tail: i32, cap: i32) -> (i32, bool) {
        let mut seen = vec![false; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        q.push_back(head);
        let mut count = 0;
        let mut reached = false;
        while let Some(c) = q.pop_front() {
            for (_sc, d) in DIRS {
                let n = c + d;
                if n < 0 || (n as usize) >= 4000 || seen[n as usize] {
                    continue;
                }
                let blocked = if soft_smiley {
                    self.hard_blocked(n)
                } else {
                    self.static_blocked(n)
                };
                if !blocked && !occ[n as usize] {
                    seen[n as usize] = true;
                    count += 1;
                    if n == tail {
                        reached = true;
                    }
                    if count >= cap {
                        return (count, reached);
                    }
                    q.push_back(n);
                }
            }
        }
        (count, reached)
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
