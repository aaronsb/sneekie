//! An AlphaGo-shaped MCTS planner for the Sneekie+ danger phase.
//!
//! When the swarm is loose the board is the one place lookahead truly pays:
//! hunters are a quasi-adversary, and (because no new faces spawn while they
//! hunt) their motion is *deterministic* given the snake's moves — so it can be
//! searched. This is Monte-Carlo Tree Search in the AlphaGo mould, minus the
//! learned nets: **PUCT** selection, a hand-rolled **policy prior** (a softmax
//! over food-proximity and hunter-distance) standing in for the policy network,
//! and **hunter-aware rollouts** scored by survival-and-eating standing in for
//! the value network. It returns one move per tick — the swarm moves every tick,
//! so the bot replans from scratch each time rather than committing a plan.
//!
//! Several cores of an 8086's worth of compute, finally, brought to bear on a
//! 1988 GW-BASIC snake.

use std::collections::{HashSet, VecDeque};

use super::sim::{Outcome, Sim};
use super::DIRS;

/// Tree simulations per move — the search budget (the "several cores" knob).
pub(super) const MCTS_SIMS: u32 = 1200;
/// How many moves each rollout plays before scoring the resulting state.
const ROLLOUT_DEPTH: usize = 35;
/// PUCT exploration constant: higher widens the search, lower sharpens it.
const C_PUCT: f64 = 1.3;

struct Node {
    sim: Sim,
    parent: usize,                 // index in the arena (root points to itself)
    mv: u32,                       // scan code of the move from the parent
    prior: f64,                    // policy prior P(a) for that move
    children: Vec<usize>,          // expanded child indices
    untried: Vec<(u32, i32, f64)>, // (scan code, offset delta, prior) not yet expanded
    n: u32,                        // visit count
    w: f64,                        // accumulated value
    terminal: Option<f64>,         // Some(value) once dead (0.0) or cleared (1.0)
}

/// Map a rolled-out state to [0, 1]. The winning strategy under the swarm is to
/// **out-eat it**: every heart banks points, the bank funds ram-kills, and
/// clearing the hearts ends the level — so *eating* is the dominant productive
/// term. Survival is the gate beneath it (a line that dies early keeps only a
/// fraction of the survival credit), and proximity to the nearest heart breaks
/// ties so the search closes on food rather than twirling in a safe pocket.
fn value(steps: usize, eaten: i32, start_food: usize, end_dist: i32) -> f64 {
    let surv = steps as f64 / ROLLOUT_DEPTH as f64;
    let eat = if start_food > 0 {
        (eaten as f64 / start_food as f64).min(1.0)
    } else {
        0.0
    };
    let prox = if end_dist == i32::MAX {
        0.0
    } else {
        1.0 - (end_dist.min(60) as f64 / 60.0)
    };
    (0.30 * surv + 0.50 * eat + 0.20 * prox).clamp(0.0, 1.0)
}

impl crate::game::Game {
    /// Static-obstacle map for the danger phase. Identical to the planner's, but
    /// hunters (`2`) are left *passable* — they're tracked dynamically in the
    /// `Sim` so the search can move them, not frozen as walls.
    fn build_blocked_dynamic(&self) -> Vec<bool> {
        let mut b = vec![false; 4000];
        for off in (0..4000i32).step_by(2) {
            b[off as usize] = self.static_blocked(off) && self.peek(off) != 2;
        }
        b
    }

    /// Snapshot the live danger-phase board into a `Sim`: body, food, the current
    /// hunters, and the score available to spend on ram/defeat costs.
    fn build_danger_sim(&self) -> Sim {
        let mut body: VecDeque<i32> = VecDeque::new();
        for i in self.etel..=self.btel {
            body.push_back(self.t[i as usize]);
        }
        let mut food: HashSet<i32> = HashSet::new();
        for off in (0..4000i32).step_by(2) {
            if matches!(self.peek(off), 3 | 5) {
                food.insert(off);
            }
        }
        let mut sim = Sim::new(body, food);
        sim.set_hunters(self.hunters.clone(), self.zcore);
        sim
    }

    /// Build a fresh node: compute the legal (non-reverse) moves and a softmax
    /// policy prior over them from food-proximity and hunter-distance.
    fn mcts_node(&self, sim: Sim, parent: usize, mv: u32, prior: f64, dist: &[i32]) -> Node {
        let rev = -sim.last_dir();
        let head = sim.head();
        let mut raw: Vec<(u32, i32, f64)> = Vec::new();
        for (sc, d) in DIRS {
            if d == rev {
                continue;
            }
            let n = head + d;
            let fd = if n >= 0 && (n as usize) < 4000 {
                let v = dist[n as usize];
                if v == i32::MAX {
                    300
                } else {
                    v
                }
            } else {
                300
            };
            let hd = sim.nearest_hunter_dist(n).min(20);
            // Prefer cells closer to food and farther from hunters.
            raw.push((sc, d, -(fd as f64) + 2.0 * hd as f64));
        }
        let maxr = raw.iter().map(|x| x.2).fold(f64::MIN, f64::max);
        let sum: f64 = raw.iter().map(|x| (x.2 - maxr).exp()).sum();
        let untried: Vec<(u32, i32, f64)> = raw
            .iter()
            .map(|&(sc, d, r)| (sc, d, (r - maxr).exp() / sum))
            .collect();
        Node {
            sim,
            parent,
            mv,
            prior,
            children: Vec::new(),
            untried,
            n: 0,
            w: 0.0,
            terminal: None,
        }
    }

    /// PUCT: pick the child maximizing Q + c·P·√N_parent / (1 + N_child).
    fn mcts_select(&self, arena: &[Node], ni: usize) -> usize {
        let sqrt_parent = (arena[ni].n.max(1) as f64).sqrt();
        let mut best = (f64::MIN, arena[ni].children[0]);
        for &ci in &arena[ni].children {
            let c = &arena[ci];
            let q = if c.n > 0 { c.w / c.n as f64 } else { 0.0 };
            let u = C_PUCT * c.prior * sqrt_parent / (1.0 + c.n as f64);
            if q + u > best.0 {
                best = (q + u, ci);
            }
        }
        best.1
    }

    /// A heavy (heuristic) rollout: greedily flee hunters / approach food until
    /// death, clear, or depth — then score the result.
    fn mcts_rollout(&self, mut sim: Sim, blocked: &[bool], dist: &[i32]) -> f64 {
        let start_food = sim.food_left();
        let mut eaten = 0;
        for step in 0..ROLLOUT_DEPTH {
            if sim.food_left() == 0 {
                return 1.0; // cleared the level
            }
            let rev = -sim.last_dir();
            let head = sim.head();
            let mut best: Option<(f64, i32)> = None;
            for (_sc, d) in DIRS {
                if d == rev {
                    continue;
                }
                let n = head + d;
                if !sim.enterable(n, blocked) {
                    continue;
                }
                let fd = if (n as usize) < 4000 && dist[n as usize] != i32::MAX {
                    dist[n as usize]
                } else {
                    300
                };
                let hd = sim.nearest_hunter_dist(n).min(20);
                let score = -(fd as f64) + 2.0 * hd as f64;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, d));
                }
            }
            match best {
                None => return value(step, eaten, start_food, dist[head as usize]), // boxed in
                Some((_, d)) => match sim.apply_danger(d, blocked) {
                    Outcome::Dead => return value(step, eaten, start_food, dist[head as usize]),
                    Outcome::Ate => eaten += 1,
                    // No smileys exist once the swarm is loose, so apply_danger
                    // never yields Penalty; handle it for exhaustiveness.
                    Outcome::Moved | Outcome::Penalty => {}
                },
            }
        }
        value(ROLLOUT_DEPTH, eaten, start_food, dist[sim.head() as usize])
    }

    /// Run MCTS for the current danger-phase board; return the best move (the
    /// most-visited root child), or `None` if there's nothing to search.
    pub(super) fn plan_mcts(&self) -> Option<u32> {
        let blocked = self.build_blocked_dynamic();
        let dist = self.food_dist_field(&blocked);
        let root = self.build_danger_sim();
        if root.food_left() == 0 {
            return None;
        }

        let mut arena: Vec<Node> = Vec::with_capacity(MCTS_SIMS as usize + 8);
        arena.push(self.mcts_node(root, 0, 0, 1.0, &dist));

        for _ in 0..MCTS_SIMS {
            // --- selection: descend by PUCT to a node with room to grow ---
            let mut ni = 0usize;
            while arena[ni].terminal.is_none()
                && arena[ni].untried.is_empty()
                && !arena[ni].children.is_empty()
            {
                ni = self.mcts_select(&arena, ni);
            }

            // --- expansion + simulation: value of the reached/created leaf ---
            let leaf_value = if let Some(v) = arena[ni].terminal {
                v
            } else if let Some((sc, d, prior)) = arena[ni].untried.pop() {
                let mut sim = arena[ni].sim.clone();
                let out = sim.apply_danger(d, &blocked);
                let child = if out == Outcome::Dead {
                    let mut node = self.mcts_node(sim, ni, sc, prior, &dist);
                    node.terminal = Some(0.0);
                    node
                } else if sim.food_left() == 0 {
                    let mut node = self.mcts_node(sim, ni, sc, prior, &dist);
                    node.terminal = Some(1.0);
                    node
                } else {
                    self.mcts_node(sim, ni, sc, prior, &dist)
                };
                let cidx = arena.len();
                let v = child
                    .terminal
                    .unwrap_or_else(|| self.mcts_rollout(child.sim.clone(), &blocked, &dist));
                arena.push(child);
                arena[ni].children.push(cidx);
                ni = cidx;
                v
            } else {
                arena[ni].terminal.unwrap_or(0.0)
            };

            // --- backpropagation ---
            let mut cur = ni;
            loop {
                arena[cur].n += 1;
                arena[cur].w += leaf_value;
                if cur == 0 {
                    break;
                }
                cur = arena[cur].parent;
            }
        }

        // Commit to the most-visited root child (ties broken by mean value).
        let mut best: Option<(u32, u32, f64)> = None;
        for &ci in &arena[0].children {
            let c = &arena[ci];
            let q = if c.n > 0 { c.w / c.n as f64 } else { 0.0 };
            let take = match best {
                None => true,
                Some((_, bn, bq)) => c.n > bn || (c.n == bn && q > bq),
            };
            if take {
                best = Some((c.mv, c.n, q));
            }
        }
        best.map(|(mv, _, _)| mv)
    }
}
