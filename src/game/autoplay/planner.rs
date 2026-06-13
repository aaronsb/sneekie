//! The bounded forward-search planner for autoplay.
//!
//! Several cores of modern compute against an 8086 GW-BASIC game: this snapshots
//! the live board into a [`Sim`] and runs a budget-capped **beam search** over
//! move sequences, rolling each candidate forward and scoring the resulting
//! state. It returns the move sequence (scan codes) to the best state it found;
//! the bot queues that plan and executes it tick-by-tick, replanning when the
//! plan empties, the board reseeds (a heart was eaten), or the next queued step
//! is no longer legal.
//!
//! Scoring, in priority order: food eaten (dominant), whether the head can still
//! reach its tail (the immortality guarantee), distance to the nearest remaining
//! food (pulls the search toward heart clusters), and open space (a tiebreak).
//!
//! Lookahead only pays off where the world is static, so the caller restricts
//! the planner to the static maze levels — Sneekie+ hunters and the moving-arrow
//! levels fall through to the per-tick greedy chain instead.

use std::collections::{HashSet, VecDeque};

use super::sim::{Outcome, Sim};
use super::{reverse, DIRS};

/// Hard cap on simulated steps per replan — the "bailout after N simulations".
/// Rarely reached: a typical replan expands only beam_width × depth × 4 nodes.
pub(super) const PLAN_BUDGET: i32 = 5000;
/// How many moves deep the beam looks (far enough to see across a cleared patch
/// to the next heart cluster).
pub(super) const PLAN_DEPTH: usize = 30;
/// Beam width: survivors kept after each depth layer.
pub(super) const BEAM_WIDTH: usize = 6;
/// Score docked per smiley the plan eats through. Far below a heart's reward
/// (1,000,000) so the planner *will* punch through a smiley to reach food, but
/// well above the distance term so it only does so when there's no clean path.
const PENALTY_WEIGHT: i64 = 200_000;
/// Score docked per step taken — time is the real pressure: every move drains
/// Bonus, and dithering during the Sneekie+ grace only brings the swarm sooner.
/// Kept *below* `DIST_WEIGHT` so a step that gets closer to food still nets
/// positive while a step that doesn't (snaking back and forth) nets negative.
const STEP_PENALTY: i64 = 200;
/// Weight on distance-to-nearest-food. Each step closer is worth this; it must
/// exceed `STEP_PENALTY` for the bot to march toward far food instead of idling.
const DIST_WEIGHT: i64 = 500;
/// Weight on reachable free space (maneuvering room). Heavy enough that the bot
/// won't pack its tail into a small pocket to stay "tail-safe" — keeping the
/// board open beats a marginally shorter route to the next heart.
const OPEN_WEIGHT: i64 = 150;

struct Node {
    sim: Sim,
    path: Vec<u32>,
    eaten: i32,
    penalties: i32,
}

impl crate::game::Game {
    /// The hard-obstacle map (true = impassable) read from VRAM: walls, stones,
    /// arrows, hunters. Smileys are deliberately *left out* — they're passable at
    /// a cost (see [`super::sim::Sim`]), so a smiley can't wall off a region.
    fn build_blocked(&self) -> Vec<bool> {
        let mut blocked = vec![false; 4000];
        for off in (0..4000i32).step_by(2) {
            blocked[off as usize] = self.hard_blocked(off);
        }
        blocked
    }

    /// Snapshot the live board into a [`Sim`]: the body (tail→head), remaining
    /// heart/club cells, and the smiley cells the plan may eat through at a cost.
    fn build_sim(&self) -> Sim {
        let mut body: VecDeque<i32> = VecDeque::new();
        for i in self.etel..=self.btel {
            body.push_back(self.t[i as usize]);
        }
        let mut food: HashSet<i32> = HashSet::new();
        let mut penalty: HashSet<i32> = HashSet::new();
        let mut stones: HashSet<i32> = HashSet::new();
        for off in (0..4000i32).step_by(2) {
            match self.peek(off) {
                3 | 5 => {
                    food.insert(off);
                }
                1 => {
                    penalty.insert(off);
                }
                10 => {
                    stones.insert(off);
                }
                _ => {}
            }
        }
        let mut sim = Sim::new(body, food);
        sim.set_penalty(penalty);
        sim.set_stones(stones);
        sim
    }

    /// Multi-source BFS from every food cell, ignoring the snake body, giving a
    /// distance-to-nearest-food for any cell in O(1). The body-ignoring
    /// approximation is fine — this only *guides* the beam toward clusters; the
    /// per-node tail-safety check is what actually keeps moves survivable.
    pub(super) fn food_dist_field(&self, blocked: &[bool]) -> Vec<i32> {
        let mut dist = vec![i32::MAX; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        for off in (0..4000).step_by(2) {
            if matches!(self.peek(off as i32), 3 | 5) {
                dist[off] = 0;
                q.push_back(off as i32);
            }
        }
        while let Some(c) = q.pop_front() {
            let dc = dist[c as usize];
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0
                    && (n as usize) < 4000
                    && !blocked[n as usize]
                    && dist[n as usize] == i32::MAX
                {
                    dist[n as usize] = dc + 1;
                    q.push_back(n);
                }
            }
        }
        dist
    }

    fn score_node(
        &self,
        sim: &Sim,
        blocked: &[bool],
        dist: &[i32],
        eaten: i32,
        penalties: i32,
        depth: i32,
    ) -> i64 {
        let mut s = eaten as i64 * 1_000_000;
        s -= penalties as i64 * PENALTY_WEIGHT;
        if sim.tail_reachable(blocked) {
            s += 100_000;
        }
        let dh = dist[sim.head() as usize];
        let dp = if dh == i32::MAX { 300 } else { dh.min(300) };
        s -= dp as i64 * DIST_WEIGHT;
        // Time pressure: every step spent burns Bonus, so a step is only worth it
        // if it brings the head closer to food. STEP_PENALTY < DIST_WEIGHT, so
        // progress nets positive and twirling-in-place nets negative.
        s -= depth as i64 * STEP_PENALTY;
        s += sim.open_space(blocked) as i64 * OPEN_WEIGHT;
        s
    }

    /// Beam search; returns the move sequence to the best state found, or `None`
    /// if nothing survives the first ply (then the greedy fallback drives).
    pub(super) fn plan_path(&self) -> Option<VecDeque<u32>> {
        let blocked = self.build_blocked();
        let init = self.build_sim();
        if init.food_left() == 0 {
            return None; // level effectively cleared — nothing to route toward
        }
        let dist = self.food_dist_field(&blocked);

        let mut beam = vec![Node { sim: init, path: Vec::new(), eaten: 0, penalties: 0 }];
        let mut best: Option<(i64, Vec<u32>, i32)> = None;
        let mut budget = PLAN_BUDGET;

        'depth: for _ in 0..PLAN_DEPTH {
            let mut next: Vec<(i64, Node)> = Vec::new();
            for node in beam.drain(..) {
                for (sc, d) in DIRS {
                    if budget <= 0 {
                        break 'depth;
                    }
                    budget -= 1;
                    let mut sim = node.sim.clone();
                    let out = sim.apply(d, &blocked);
                    if out == Outcome::Dead {
                        continue;
                    }
                    let eaten = node.eaten + i32::from(out == Outcome::Ate);
                    let penalties = node.penalties + i32::from(out == Outcome::Penalty);
                    let mut path = node.path.clone();
                    path.push(sc);
                    let score =
                        self.score_node(&sim, &blocked, &dist, eaten, penalties, path.len() as i32);
                    if best.as_ref().is_none_or(|(b, _, _)| score > *b) {
                        best = Some((score, path.clone(), eaten));
                    }
                    next.push((score, Node { sim, path, eaten, penalties }));
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort_by(|a, b| b.0.cmp(&a.0));
            next.truncate(BEAM_WIDTH);
            beam = next.into_iter().map(|(_, n)| n).collect();
        }
        // If the beam reaches food within its horizon, commit that plan. If it
        // doesn't (the local area is cleared), don't dither — descend the
        // whole-board food-distance field straight toward the next region with
        // points. Falling all the way through means truly boxed in → let greedy
        // and the wedge guard handle it.
        if let Some((_, path, eaten)) = &best {
            if *eaten > 0 {
                return Some(path.clone().into_iter().collect());
            }
        }
        if let Some(mv) = self.beeline_move() {
            return Some(VecDeque::from([mv]));
        }
        best.map(|(_, path, _)| path.into_iter().collect())
    }

    /// Body-aware BFS to the nearest remaining heart, returning the first step of
    /// that shortest path. Used when the beam can't reach food within its horizon
    /// (the local patch is cleared): a real path over open cells — smileys
    /// passable at their cost, the snake's own body NOT — so a long snake heads
    /// straight for the next region instead of oscillating against a body-blind
    /// distance gradient.
    fn beeline_move(&self) -> Option<u32> {
        let head = self.t[self.btel as usize];
        let rev = reverse(self.e);
        let mut seen = vec![false; 4000];
        let mut first = vec![0u32; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && matches!(self.peek(n), 32 | 3 | 5 | 1 | 10) {
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
                if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && matches!(self.peek(n), 32 | 3 | 5 | 1 | 10) {
                    seen[n as usize] = true;
                    first[n as usize] = first[c as usize];
                    q.push_back(n);
                }
            }
        }
        None
    }
}
