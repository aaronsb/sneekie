//! A throwaway forward-simulation of the board, for autoplay lookahead.
//!
//! [`Sim`] is a cheap, mutable copy of just what the planner needs to roll the
//! game forward without touching live VRAM: the snake body (a deque, tail at the
//! front, head at the back), an occupancy set for O(1) self-collision tests, and
//! the set of remaining food cells. Static obstacles (walls, stones, smileys,
//! arrows and — in Sneekie+ — hunters) live in an immutable `blocked` map the
//! planner builds once per replan and threads through by reference, so cloning a
//! `Sim` only copies the small mutable parts.

use std::collections::{HashSet, VecDeque};

use super::DIRS;

/// Points it costs to defeat a hunter on contact — must match `HUNTER_COST` in
/// [`super::super::plus`]; mirrored here so the danger-phase rollout can model
/// ram affordability without reaching across modules.
const HUNTER_COST: i32 = 75;

/// What one simulated step did.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum Outcome {
    Moved,
    Ate,     // a heart/club: grow, score up
    Penalty, // a smiley: grow, score down (taken only when it's the way through)
    Dead,
}

#[derive(Clone)]
pub(super) struct Sim {
    body: VecDeque<i32>,  // front = tail, back = head
    occ: HashSet<i32>,    // body occupancy (fast collision tests)
    food: HashSet<i32>,   // remaining heart/club cells
    penalty: HashSet<i32>, // smiley cells: passable, but eating one costs -50
    hunters: Vec<i32>,    // Sneekie+ danger phase: hunter offsets (empty otherwise)
    hocc: HashSet<i32>,   // hunter occupancy (fast tests)
    wallet: i32,          // banked score available to pay ram/defeat costs
}

impl Sim {
    pub(super) fn new(body: VecDeque<i32>, food: HashSet<i32>) -> Self {
        let occ: HashSet<i32> = body.iter().copied().collect();
        Sim {
            body,
            occ,
            food,
            penalty: HashSet::new(),
            hunters: Vec::new(),
            hocc: HashSet::new(),
            wallet: 0,
        }
    }

    /// Record the smiley cells the planner may eat through at a cost.
    pub(super) fn set_penalty(&mut self, penalty: HashSet<i32>) {
        self.penalty = penalty;
    }

    /// Load the Sneekie+ danger-phase state: the live hunters and the score the
    /// snake can spend defeating them. Enables [`Sim::apply_danger`].
    pub(super) fn set_hunters(&mut self, hunters: Vec<i32>, wallet: i32) {
        self.hocc = hunters.iter().copied().collect();
        self.hunters = hunters;
        self.wallet = wallet;
    }

    pub(super) fn head(&self) -> i32 {
        *self.body.back().unwrap()
    }

    pub(super) fn food_left(&self) -> usize {
        self.food.len()
    }

    /// The offset delta of the last move (head − neck), or 0 for a length-1 body.
    /// Used to forbid reversing into the neck.
    pub(super) fn last_dir(&self) -> i32 {
        let n = self.body.len();
        if n >= 2 {
            self.body[n - 1] - self.body[n - 2]
        } else {
            0
        }
    }

    /// Could the snake step into `off` this move? In bounds, not a static
    /// obstacle, not its own body — and a hunter cell only if it can be paid.
    pub(super) fn enterable(&self, off: i32, blocked: &[bool]) -> bool {
        if off < 0 || (off as usize) >= blocked.len() || blocked[off as usize] {
            return false;
        }
        if self.occ.contains(&off) {
            return false;
        }
        if self.hocc.contains(&off) {
            return self.wallet >= HUNTER_COST;
        }
        true
    }

    /// Manhattan distance from `off` to the nearest hunter (`i32::MAX` if none) —
    /// a cheap fear gradient for the rollout policy.
    pub(super) fn nearest_hunter_dist(&self, off: i32) -> i32 {
        let (r, c) = (off / 160, (off % 160) / 2);
        self.hunters
            .iter()
            .map(|&h| (h / 160 - r).abs() + ((h % 160) / 2 - c).abs())
            .min()
            .unwrap_or(i32::MAX)
    }

    /// Roll one step by VRAM-offset delta `d`. `blocked` is the static-obstacle
    /// map (true = impassable). Mutates the sim and reports what happened.
    pub(super) fn apply(&mut self, d: i32, blocked: &[bool]) -> Outcome {
        let next = self.head() + d;
        if next < 0 || (next as usize) >= blocked.len() || blocked[next as usize] {
            return Outcome::Dead;
        }
        let is_food = self.food.contains(&next);
        let is_pen = self.penalty.contains(&next);
        // Eating a heart *or* a smiley grows the snake (the head advances, the
        // tail stays) — matching the BASIC. Only an empty step vacates the tail,
        // so stepping into the cell the tail is leaving is legal.
        let grow = is_food || is_pen;
        if !grow {
            if let Some(tail) = self.body.pop_front() {
                self.occ.remove(&tail);
            }
        }
        if self.occ.contains(&next) {
            return Outcome::Dead; // ran into the body (incl. reversing into the neck)
        }
        self.occ.insert(next);
        self.body.push_back(next);
        if is_food {
            self.food.remove(&next);
            Outcome::Ate
        } else if is_pen {
            self.penalty.remove(&next);
            Outcome::Penalty
        } else {
            Outcome::Moved
        }
    }

    /// A cell the snake could move through right now: in bounds, not a static
    /// obstacle, not currently part of the body.
    fn free(&self, off: i32, blocked: &[bool]) -> bool {
        off >= 0
            && (off as usize) < blocked.len()
            && !blocked[off as usize]
            && !self.occ.contains(&off)
    }

    /// Can the head still reach its own tail over free cells? If so the snake can
    /// always escape by following its tail, so it can't have boxed itself in.
    /// Early-exits on success and is capped, so a hopeless search stays cheap
    /// (returning `false` — conservatively "unsafe").
    pub(super) fn tail_reachable(&self, blocked: &[bool]) -> bool {
        let head = self.head();
        let tail = *self.body.front().unwrap();
        if head == tail {
            return true;
        }
        let mut seen: HashSet<i32> = HashSet::new();
        let mut q: VecDeque<i32> = VecDeque::new();
        seen.insert(head);
        q.push_back(head);
        let mut visited = 0;
        while let Some(c) = q.pop_front() {
            visited += 1;
            if visited > 900 {
                return false;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n == tail {
                    return true;
                }
                if self.free(n, blocked) && seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        false
    }

    /// Reachable free space from the head (capped) — the bot's maneuvering room.
    /// Weighted heavily by the planner so it won't coil its tail into a small
    /// pocket (which keeps the tail "reachable" yet boxes the head in). The cap
    /// is generous enough to tell a cramped coil from an open board.
    pub(super) fn open_space(&self, blocked: &[bool]) -> i32 {
        let head = self.head();
        let mut seen: HashSet<i32> = HashSet::new();
        let mut q: VecDeque<i32> = VecDeque::new();
        seen.insert(head);
        q.push_back(head);
        let mut count = 0;
        while let Some(c) = q.pop_front() {
            count += 1;
            if count > 500 {
                break;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if self.free(n, blocked) && seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        count
    }

    /// One Sneekie+ danger-phase step: move the head (ramming a hunter if it can
    /// pay), then advance every hunter one cell toward the new head — exactly as
    /// [`super::super::plus::Game::plus_hunters`] does in the live game. Because
    /// no new faces spawn while the swarm is loose, this rollout is faithful.
    pub(super) fn apply_danger(&mut self, d: i32, blocked: &[bool]) -> Outcome {
        let next = self.head() + d;
        if next < 0 || (next as usize) >= blocked.len() || blocked[next as usize] {
            return Outcome::Dead;
        }
        let mut eating = false;
        if self.hocc.contains(&next) {
            // Ram a hunter: pay to clear it (tail moves, like an empty step), or die.
            if self.wallet < HUNTER_COST {
                return Outcome::Dead;
            }
            self.wallet -= HUNTER_COST;
            self.hunters.retain(|&h| h != next);
            self.hocc.remove(&next);
        } else {
            eating = self.food.contains(&next);
        }
        if !eating {
            if let Some(tail) = self.body.pop_front() {
                self.occ.remove(&tail);
            }
        }
        if self.occ.contains(&next) {
            return Outcome::Dead; // ran into the body
        }
        self.occ.insert(next);
        self.body.push_back(next);
        if eating {
            self.food.remove(&next);
        }
        if self.step_hunters(next, blocked) {
            return Outcome::Dead; // a hunter caught the head and we couldn't pay
        }
        if eating {
            Outcome::Ate
        } else {
            Outcome::Moved
        }
    }

    /// Advance hunters toward `head`. Returns `true` if one reaches the head and
    /// can't be paid (death). Mirrors the live chase rule: step along the axis of
    /// greater distance into empty cells only; walls, food, body, and other
    /// hunters block; reaching the head pays the cost or kills.
    fn step_hunters(&mut self, head: i32, blocked: &[bool]) -> bool {
        let (hr, hc) = (head / 160, (head % 160) / 2);
        let current = std::mem::take(&mut self.hunters);
        let mut survivors: Vec<i32> = Vec::with_capacity(current.len());
        for off in current {
            self.hocc.remove(&off); // it's about to move, stay, or be paid off
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
                self.hocc.insert(off);
                survivors.push(off);
            } else if new == head {
                if self.wallet < HUNTER_COST {
                    return true; // caught, can't pay
                }
                self.wallet -= HUNTER_COST; // defeated on contact, vanishes
            } else if new >= 0
                && (new as usize) < blocked.len()
                && !blocked[new as usize]
                && !self.occ.contains(&new)
                && !self.hocc.contains(&new)
                && !self.food.contains(&new)
            {
                self.hocc.insert(new);
                survivors.push(new);
            } else {
                self.hocc.insert(off); // blocked: hold position
                survivors.push(off);
            }
        }
        self.hunters = survivors;
        false
    }
}
