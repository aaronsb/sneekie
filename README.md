# Sneekie

A faithful **terminal** port of *Sneekie* — a snake game written in GW-BASIC in
July 1988 by Herbert Groot Jebbink (HerbySoft) and published on the MSX/MS-DOS
Computer Magazine diskette MCMPC-D2 (no. 25, October 1988).

The original ran in 80×25 text mode and POKE'd characters straight into video
memory. This port keeps that model exactly — a 4000-byte "VRAM" buffer rendered
through the **CP437** character set — so the box-drawing snake, the `♥`/`♣`
food, the `☺` traps, the `◙` pushable stones and the `↑→←` arrow hazards all
look just like 1988, in any modern terminal.

Ported from the [single-page HTML re-creation](https://herbert256.github.io/sneekie/)
of the original `SNEEKIE.BAS` — by Herbert Groot Jebbink, whose source lives at
[github.com/herbert256/sneekie](https://github.com/herbert256/sneekie). Code
comments carry the original BASIC line numbers so the lineage stays legible.

The port is faithful, but the substance that grew here is the **self-playing
bot** — a study in making a game-playing algorithm competent enough that you stop
catching its mistakes. If you only read one part, read [Autoplay](#autoplay-the-bot--screensaver)
and the [notes on building it](#notes-from-building-the-player).

## Build & run

```sh
cargo run --release                            # boot menu: pick mode + movement
cargo run --release -- --classic --turn-based  # the 1988 feel
cargo run --release -- --plus --live           # survival, always gliding
cargo run --release -- --auto                  # watch the bot play
cargo run --release -- --auto --plus           # watch the bot survive the swarm
cargo run --release -- --auto --plus --planner-cores 8  # parallel MCTS
cargo run --release -- --theme amber
cargo run --release --features audio -- --plus # with real square-wave sound
```

Needs an **80×25** (or larger) terminal. If the window is too small it'll ask
you to enlarge it.

## Two axes: mode × movement

Launching with no flags shows a two-step CP437 boot menu. You pick a **mode**
and a **movement style**, independently — four combinations in all (or `A` for
autoplay, below).

**Mode** (`--classic` / `--plus`):
- **Classic** — no hunters; just clear the hearts.
- **Sneekie+** — survival mode with hunters (below).

**Movement** (`--turn-based` / `--live`):
- **Turn-based** — the snake steps once per keypress. In Sneekie+ the hunters
  move in lockstep with you, one step each time you step — pure tactical chess.
- **Live** — the snake is always gliding (the modern snake feel). In Sneekie+
  the hunters step on every glide-tick.

(With only one axis given as a flag, the other defaults sensibly: `--plus`
implies live, `--classic` implies turn-based.)

### Sneekie+ survival mode

Sneekie+ is a **wave system** layered over the level: a cycle between calm
heart-collecting and a "zombie" swarm.

- **Grace period.** Each wave opens calm — smileys are static `−50` traps and
  the field keeps seeding faces as you eat. **Bank as many points as you can**;
  the clock is shown on the spare bottom row. Grace is generous for the first
  wave (**25s**) and **tightens every cycle** (the swarm returns faster), down
  to an 8s floor.
- **The swarm wakes.** When the timer expires, the nearest smileys become
  **hunters** (`☻`, glowing red), the rest clear out, and **no new faces spawn**.
  Hunters step one cell toward your head on **every move** you make (in
  lockstep, if you're playing turn-based). Walls block them — use cover.
- **Banked points are your shield.** A hunter touching you — or you ramming one —
  **costs points and removes that hunter**, *if you can afford it* (the price is
  shown as "ram-kills" you can buy). Can't pay? It's death. So the danger phase
  is a race: keep eating faster than the hunters can reach you.
- **A score multiplier** climbs while the swarm is loose — ×2 the moment they
  wake, +1 each second — scaling the points you earn (not what you pay).
- **Clear the wave** (defeat every hunter) and you earn a breather: faces return,
  the multiplier resets, and a fresh — shorter — clock counts down to the next
  wave. Survive the cycles long enough to clear all the hearts and finish the
  level.

The field is thinned in Sneekie+ (vs. classic's dense scatter) so a level is
actually winnable under the pressure. Press **`m`** to toggle sound.

Balance lives in named constants at the top of `src/game/plus.rs`
(`GRACE_BASE`/`GRACE_STEP`/`GRACE_MIN`, `HUNTERS_MAX`, `HUNTER_COST`,
`WAVE_FACES`) and `glide_speed` in `src/game/play.rs` — easy to dial.

### Autoplay (the bot / screensaver)

`--auto` (or `A` in the menu, which then asks classic or Sneekie+) hands the
snake to a bot that plays — and, with its auto-restart, basically turns Sneekie
into a weird little screensaver (locked to glorious CGA color).

It picks a **brain** per tick to suit the situation — several cores of modern
compute thrown at an 8086 GW-BASIC game:

**Beam planner — the static maze levels** (classic, and the Sneekie+ grace
period before the swarm wakes). Each replan snapshots the board into a throwaway
`Sim` and runs a budget-capped **beam search** over move sequences, rolling each
candidate forward and scoring the result by — in priority order — food eaten,
**smileys eaten** (a cost, see below), whether the head can still reach its tail,
distance to the nearest heart, and open space. It commits to the best plan and
executes it tick-by-tick (at full speed, not game-tick speed), bailing out after
a fixed number of simulated steps. Every committed move passes a **hard
tail-safety gate**, so the planner supplies the routing while a proven one-ply
check supplies the never-self-trap guarantee.

**MCTS — the Sneekie+ danger phase**, when the swarm is loose. Hunters are a
quasi-adversary, but because no new faces spawn while they hunt, their motion is
*deterministic* given the snake's moves — so it can be searched. This is
**Monte-Carlo Tree Search in the AlphaGo mould**, minus the learned nets: PUCT
selection, a hand-rolled **policy prior** (a softmax over food-proximity and
hunter-distance) standing in for the policy network, and **hunter-aware
rollouts** scored by survival-and-eating standing in for the value network. The
rollout simulates the swarm forward exactly (the same one-step-toward-the-head
chase rule the live game uses), so a cell that's safe now but lethal in three
moves is seen as lethal. It returns one move per tick and replans from scratch
the next — the swarm has moved, after all.

The MCTS is **root-parallel**: `--planner-cores N` (or `+`/`-` live while it
plays; the danger-phase HUD shows cores + tree nodes searched) grows N
independent trees from the same root, each with its own seeded ε-greedy rollouts
*and* per-worker root noise so they deepen **different** first-move lines, then
merges their visit counts. The default reserves a couple of hardware threads.

There's a real lesson in the tuning here. With **shallow** search, more cores do
nothing: each per-move decision has a branching factor of ≤3, so one core's MCTS
already saturates it and the extra trees just agree. The compute only pays once
it's spent on **depth** — long rollouts (100-move horizon) and root-diversified
deep lines. Measured on the level-2 swarm, same wall-clock per move: 1 core lost
3 lives over the danger phase; 8 cores lost 1. Breadth saturates; depth scales —
which is the whole point of throwing several modern cores at an 8086 snake.

**Take the penalty if it's the only way through.** Smileys (`−50`) are not walls
to the planner — they're passable *at a cost*. If the only route to the remaining
hearts runs through a smiley, the bot eats it and moves on (a heart is worth far
more than a smiley costs); if there's a clean path, it takes that instead. The
same logic governs ramming hunters in Sneekie+ — pay the cost when it buys a way
forward. **And if there's no winning line at all, that's a draw**: the bot gives
up the level and respawns (F10 skip in classic — no life lost; **ESC** in
Sneekie+ — a life given up, fair and square) rather than thrashing forever.

**Time-aware planner — the moving-hazard levels.** The back-half levels run
climbing/sweeping `↑→←` arrows and wall gaps that crawl down one row per step.
These are deterministic, so the bot forecasts them and routes a *timed* path: a
breadth-first search over `(cell, step)` to the nearest heart that's collision-free
at every step — an arrow cell is lethal at exactly the steps an arrow occupies it,
and a wall cell is passable only on the steps its gap is open. It replans every
tick (the hazards have moved), threading arrow lanes and crossing gaps as they
line up instead of chasing an opening it can't reach in time.

When no planner is in charge — or when any search turns up nothing safe — a
per-tick **greedy chain** backs everything up: BFS to the nearest food if the step
stays tail-safe → ram an adjacent hunter it can afford → chase its own tail to
thread out of a pocket → else head for the most open space. A short **move-history
ring** also breaks repeating cycles, and two **stall/wedge guards** (no score gain
for a long stretch, or the head not actually moving for several ticks) trigger the
give-up-and-respawn above so the screensaver never freezes.

Result: effectively immortal on the static classic levels, clearing the stone and
moving-hazard levels too (it pushes stones and times the arrows/gaps), cycling the
layouts in CGA color; a fair, go-down-swinging run in Sneekie+. A fast-moving
swarm can still overwhelm it — and that's fine, it just restarts. Watch it play
either mode (`--auto`, `--auto --plus`, or the menu's
`A`). Ctrl+C to quit. (`AUTO_STALL` in `src/game/autoplay/mod.rs` tunes patience;
`PLAN_BUDGET`/`PLAN_DEPTH`/`BEAM_WIDTH` in `planner.rs` and `MCTS_SIMS` in
`mcts.rs` tune how hard the two planners think.)

### Notes from building the player

Porting the 1988 game was the easy half. The interesting half was making the bot
good enough that you stop catching its mistakes — the moment it crosses from *"why
didn't it do X?!"* to *"I'm not sure what the best move was either."* It got there
not through one clever trick but by **modeling the world it acts in**. Almost every
visible blunder turned out to be a dynamic the planner simply wasn't simulating:

| "Why didn't it…" | What it wasn't modeling | Fix |
|---|---|---|
| avoid trapping itself in its own tail | the tail vacating over time | forward sim with a real body deque |
| keep room around its head | reachable space vs. body length | head-room ≥ length safety gate |
| go grab the last few hearts | room necessarily shrinks late-game | relax the gate to a maneuvering floor |
| push a stone out of the way | stones are shovable, not walls | model the push in the sim |
| ram the *last* hunter to end a wave | clearing the wave ends the danger | reward reaching zero hunters |
| time a crawling gap / dodge an arrow | hazards have *future* positions | time-aware `(cell, step)` search |

Two findings worth keeping:

- **Brains by regime, not one universal solver.** A fully-observable game with a
  branching factor of ≤3 doesn't need a single grand algorithm; it needs the right
  *model* per situation — a tail-aware beam on static boards, an AlphaGo-shaped
  MCTS against the (deterministic, and therefore searchable) swarm, and a
  time-expanded BFS through the moving hazards.
- **Compute buys depth, not breadth.** Parallelising the MCTS across cores did
  nothing while the search was shallow — a single core already saturates a 3-way
  per-move decision. The cores only paid once the budget went into a *longer
  horizon*: 8 cores then lost 1 life where 1 core lost 3, same swarm, same frame
  rate. Several cores of modern compute against an 8086 snake — and the lesson is
  that the snake's depth, not its width, is where the compute goes.

This is also a stepping stone, not a destination: the MCTS root **visit-count
distribution is exactly the policy-training target** a learned net would want, and
a rollout's outcome is its value target — so every strong run is already a labelled
training example. The competent hand-built player is a corpus generator in waiting.

## How to play

Eat every `♥` heart to clear a level (plus every `♣` club from level 17 on).
There are 32 levels across 8 maze layouts; the back 16 add moving hazards and
auto-advancing speed.

| Key | Action |
|-----|--------|
| Arrow keys | Steer the snake |
| `ESC` | Give up a life when you're stuck |
| `m` | Toggle sound (Sneekie+) |
| any key | Continue at a prompt |
| `1`–`4` | Switch theme live (Green / Amber / White / CGA) |
| `Ctrl+C` / `Ctrl+Q` | Quit |

Scoring: `♥` +10, `♣` +25, `☺` **−50** (avoid these!), `◙` pushable stone.
A per-level **Bonus** ticks down as you play and is added to your score when you
clear the level. Highscore, theme, and mute state persist to
`$XDG_CONFIG_HOME/sneekie/state` (`~/.config/sneekie/state`).

> There may also be a couple of function keys that do… helpful things. Shh.

## Themes

`hercules` (green), `amber`, `white`, `cga` (the colorized default). Pass with
`--theme <name>`, or press `1`–`4` at any prompt.

## Project layout

| File | Role |
|------|------|
| `src/main.rs` | Bootstrap: argument parsing, save path, help |
| `src/cp437.rs` | CP437 → Unicode table (the charset) |
| `src/theme.rs` | Color themes + CGA colorizer |
| `src/game/mod.rs` | The "machine": VRAM, CRT renderer, keyboard, BASIC output primitives |
| `src/game/layouts.rs` | The eight maze builders (`lay*`) |
| `src/game/enemies.rs` | The moving hazards (`sub*`) |
| `src/game/play.rs` | Level loop, movement, death, boot sequence |
| `src/game/plus.rs` | Sneekie+ survival mode and the boot menu |
| `src/game/autoplay/` | The self-driving bot — `mod.rs` (dispatch + safety gates + stall/wedge guards), `greedy.rs` (reactive fallback chain), `planner.rs` (bounded beam search), `mcts.rs` (MCTS vs. the swarm), `navigate.rs` (time-aware routing through arrows/gaps), `sim.rs` (forward-model) |
| `src/game/audio.rs` | Square-wave synth (behind the `audio` feature) |

Classic Sneekie is fully preserved: every Sneekie+ hook in `play.rs` is guarded
by `self.plus`, and the score multiplier is a no-op (`mult == 1`) outside the
danger phase, so classic mode runs byte-identically to the pure port.

## Notes

- **Audio** is opt-in via the `audio` Cargo feature (rodio). The default build
  is dependency-free and silent; `--features audio` adds real square-wave tones
  through the existing `snd()` hooks, queued in sequence just like the original
  `SOUND` statements. Building with the feature needs ALSA dev headers on Linux
  (`pacman -S alsa-lib`); if no audio device is found at runtime the game falls
  back to silent.
- Original game © July 1988 by HerbySoft (Herbert Groot Jebbink) —
  [github.com/herbert256/sneekie](https://github.com/herbert256/sneekie). This
  port is an homage; all credit for the design is his.
