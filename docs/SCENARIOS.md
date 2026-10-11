# Game situations ("scenes") per sport

reco-minimap can recognise **dead-ball formations** – restarts and set pieces where people stand in characteristic places and hardly move – and, in basketball, **shot attempts**. This page lists, for every sport, which situations are easy to recognise, how they are recognised and what they depend on.

Switch it on with `--scenes` (command line) or the checkbox *Detect game situations* in step 4 / 5 of the interface.

**Status legend**

| | |
|---|---|
| ✅ | **Implemented.** Rule-based on tracked positions, tested on constructed scenes. |
| 🟡 | **Implemented, depends on the ball.** Works only when the ball is found at the right moment. |
| 🔧 | **Feasible, not implemented yet.** The pattern is simple; it needs writing and tuning. |
| ❌ | **Not detectable from positions.** Needs a scoreboard, referee signals or knowledge of the rules. |

> **Honest status:** all ✅ and 🟡 scenes were verified on **constructed formations only** (positive cases and the typical look-alikes that must *not* fire). They have not been tuned on real games. Expect to adjust thresholds, and expect false positives for the weaker ones (marked below).

## Your priority list

| Sport | Situation | Status |
|---|---|---|
| Basketball | Throw-in (inbound) | ✅ |
| Basketball | Free throw | ✅ |
| Basketball | Substitution | ✅ (numbers, or leaving and re-entering at a sideline) |
| Basketball | Tip-off (jump ball) | ✅ |
| Basketball | 2- and 3-point attempts | 🟡 (needs the ball leaving the shooter's hands; not whether it went in) |
| Football | Kick-off | ✅ |
| Football | Substitution | ✅ |
| Football | Penalty | ✅ |
| Football | Corner | ✅ |
| Handball | Substitution | ✅ |
| Handball | Throw-in | ✅ |
| Handball | 7 m throw | ✅ |

---

## How it works

1. Every processed frame, the detector looks at the tracked players (positions in metres, velocities).
2. A **formation** is a pattern such as "one player standing at the free-throw line, three or more in the lane, nearly everybody standing still".
3. The pattern must **hold for a moment** (1–2 s, depends on the scene) before the event fires, and it **fires once** per formation. It can fire again only after the formation broke up and a cooldown (6 s) passed.
4. The event says what, where, which team takes it, and a confidence.

**What it needs to work**

* A calibration with an error of about **0.5 m or less**. Tolerances are 1–2 m.
* Most players tracked (about 60 % or more). A formation is not recognised if half the players are missing.
* For ball-dependent scenes, a ball that is tracked at the moment it is released.

**Typical failure modes**

* *Missed:* players not detected (far side, small picture), poor calibration, a formation that lasts less than the hold time.
* *False positives:* a player standing still at the sideline during normal play looks like a throw-in taker – this is why throw-ins carry a lower confidence (0.5) unless the ball is in his hands (0.8). A goalkeeper standing still is not a scene by itself, which is why goal kicks are not implemented.

## Output

Scenes appear in three places:

* **Log:** `SITUATION free throw left (home) at 12:34 (confidence 0.85)` and a summary at the end of a run. The wording follows the chosen language.
* **JSON** (`--json`): in the `events` array of the frame in which the scene fired:

```json
{"type": "scene", "kind": "free_throw", "team": "home", "side": "l",
 "x": 5.8, "y": 7.5, "confidence": 0.85, "borderline": false, "t": 754.2}
```

`kind` is one of `tip_off`, `kickoff`, `free_throw`, `throw_in`, `corner`, `penalty`, `seven_meter`, `shot2`, `shot3`. `side` is `l` or `r` (the end of the field the scene belongs to) or `null`. `team` is the team that takes the restart or shot. `borderline` is true for shot attempts released within the position error of the 3-point line.

* **Live overlay:** a banner for a few seconds, in the interface language.
* **`reco-minimap stats`:** lists every scene with time and confidence.

---

## Basketball

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Tip-off / jump ball** | ✅ | Two opponents inside the centre circle (≤ 1.5 m), close together, standing still; nobody else within 2.4 m of the centre; the others (≥ 6 tracked) mostly calm. Holds 1 s. | A third player in the circle (a scramble) cancels it. |
| **Free throw** | ✅ | One player standing still at the free-throw line (≤ 1.1 m), ≥ 3 players in the lane region, at most one in five players moving fast. Holds 2 s. | Reports the end (`l`/`r`) and the shooter's team. |
| **Throw-in (inbound)** | ✅ | One player standing still at the sideline or baseline (≤ 0.8 m from the edge, inside or just outside), nobody else within 1.2 m, ≥ 3 players within 9 m. Holds 1.2 s. | Confidence 0.8 if the ball is in his hands or next to him, else 0.5. A player standing at the line during play can trigger it. |
| **Substitution** | ✅ | The confirmed jersey number of a slot changes, or a player last tracked at a sideline is gone for at least 3 s and someone appears at a sideline ("possible"). See [README](../README.md#jersey-numbers-and-substitutions). | Strongest with numbers and a squad list. |
| **2- or 3-point attempt** | 🟡 | The ball leaves the holder's hands (state *held* → *in flight*) and travels towards a basket (ends ≥ 2 m closer, or within 2.5 m). Classified by the shooter's position at release: ≥ 6.8 m from the basket, or in the corner strip, is a 3. Free-throw releases are excluded. | `borderline: true` if the release is within 0.8 m of the arc – the position error is about that large. Missed when the ball is not tracked at the release; with a weak ball model that is common. |
| Made or missed | ❌ | Not visible from positions. | Needs the scoreboard. A made basket is usually followed by a baseline inbound of the *other* team within seconds – a hint, not proof. |
| Timeout | 🔧 | Everybody standing still and gathered near the bench side for > 20 s. | |
| Fast break | 🔧 | Many players of both teams crossing the halfway line in one direction at speed. | |
| Foul, violations, held ball | ❌ | Rule knowledge and referee signals. | |

## Football (soccer)

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Kick-off** | ✅ | Both teams on their own halves (≥ 85 % each), all standing still, at most two players of one team inside the centre circle (9.15 m). Holds 1.5 s. | The team inside the circle takes it. Fires at the start of each half and after a goal. |
| **Penalty** | ✅ | One player standing still at the penalty spot (≤ 1.8 m), nobody else within 8.5 m, at most one other player (the goalkeeper) in the penalty area. Holds 2 s. | Reports the end and the taker's team. |
| **Corner** | ✅ | One player standing still at a corner flag (≤ 2.4 m), ≥ 5 players in the penalty area at that end, and those players moving slowly. Holds 1.5 s. | |
| **Throw-in** | ✅ | As in basketball, at a touchline, outside the corner zone (the corner is left to the corner detector). | Same confidence rules as above. |
| **Substitution** | ✅ | As above. A substitute usually enters at the halfway line, near the touchline. | |
| Goal kick | 🔧 | Taker inside the goal area, opponents outside the penalty area. | Not implemented: a goalkeeper standing still looks the same during normal play. Needs the ball. |
| Free kick (wall) | 🔧 | ≥ 3 defenders in a line within about 10 m of the ball, standing still. | |
| Goal | 🔧 | A kick-off by the *other* team follows within a minute; a scoreboard is safer. | Cannot be told from the start of a half without the clock. |
| Offside, foul, cards | ❌ | | |

## Futsal

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Kick-off** | ✅ | As in football, centre circle 3 m. | |
| **Kick-in** | ✅ | Player standing still at a sideline (the throw-in detector). | |
| **Corner** | ✅ | As in football, with ≥ 3 players in the area. | |
| **Penalty (6 m)** | ✅ | Taker at the 6 m spot (≤ 1.5 m), nobody within 4.5 m, at most one other player in the area. | |
| **Substitution** | ✅ | As above. | Flying substitutions are frequent; expect many *possible* substitutions. |
| Second penalty (10 m) | 🔧 | | |
| Timeout | 🔧 | | |

## Handball

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Throw-off (Anwurf)** | ✅ | Both teams on their own halves, standing still, the taker at the centre. | Shown as "kick-off" in the data (`kickoff`) and as "Throw-off" in the interface. |
| **7 m throw** | ✅ | Taker standing still at the 7 m mark (≤ 1.2 m), nobody else within 1.9 m, at most one other player in the 6 m area. Holds 2 s. | |
| **Throw-in** | ✅ | Player standing still at a sideline. | Corner throws, taken at the sideline near the corner, are included. |
| **Substitution** | ✅ | As above. | Flying substitutions are frequent; expect many *possible* substitutions. |
| Free throw | 🔧 | A defensive wall of ≥ 3 players about 3 m from the taker. | |
| Goal throw (Abwurf) | 🔧 | Goalkeeper standing still in the 6 m area. | Needs the ball (same problem as the football goal kick). |
| 2-minute suspension | 🔧 | One team plays with one player fewer for two minutes: count the tracked slots of that team. | |
| Timeout | 🔧 | | |

## Hockey (field)

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Centre pass / start** | ✅ | The kick-off pattern with a 5 m centre radius. | |
| **Penalty stroke** | ✅ | Taker at the 6.4 m spot (≤ 1.5 m), nobody within 4.5 m, at most one other player in the circle. | |
| **Substitution** | ✅ | As above. | Rolling substitutions are frequent. |
| Penalty corner | 🔧 | Injector on the goal line, attackers outside the circle, defenders behind the line. | |
| Long corner, free hit | 🔧 | | |

## Rugby

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Kick-off** | ✅ | Teams on their own halves, standing still, at most two players of one team within 10 m of the centre. | |
| Scrum | 🔧 | 14–16 players bound in a circle of about 4 m, nearly still. | |
| Lineout | 🔧 | Two parallel lines of players near the touchline. | |
| Penalty or conversion kick | 🔧 | A single kicker with the others standing back. | |
| Substitution | 🔧 | Needs the substitution logic tuned for 15 slots per team. | |
| Try | ❌ | Needs the ball grounded and the scoreboard. | |

## Lacrosse

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| **Face-off** | ✅ | Two opponents at the centre (≤ 1.2 m), others standing around. | |
| Substitution | 🔧 | On-the-fly substitutions through the substitution area. | |
| Goal, man-up | ❌ | | |

## American football

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| Line of scrimmage / snap formation | 🔧 | Two facing lines of players in a band about 10 yards wide, nearly still for ≥ 5 s. A clear pattern; a good next step. | |
| Kick-off | 🔧 | A line of kickers across the field, receivers deep. | |
| Punt, field goal formations | 🔧 | | |
| Timeout | 🔧 | | |
| Touchdown, first down | ❌ | | |

## Volleyball

| Situation | Status | How it is recognised | Notes |
|---|---|---|---|
| Serve | 🔧 | One player alone behind the end line, standing still. | |
| Substitution | 🔧 | Players at the substitution zone. | |
| Rotation | 🔧 | Clockwise position change after a side-out. | |
| Point | ❌ | | |

## Other sports (free rectangle)

| Situation | Status |
|---|---|
| Throw-in / restart at the sideline | ✅ |
| Substitution | ✅ |
| Everything else | ❌ (no sport-specific geometry) |

---

## For developers: adding a scene

The detector lives in [`src/scenes.rs`](../src/scenes.rs). A scene is a function that looks at the players of one frame and pushes a candidate (`kind`, `key`, `team`, `pos`, `side`, `confidence`). The shared timing code handles the hold time, firing once, the cooldown and re-arming. Steps:

1. Add a `SceneKind` variant, its `key()` string and its `hold()` time.
2. Write the pattern function (see `free_throws` or `corners` as templates) and call it for the right sports in `patterns()`.
3. Add a test in `tests/scenes.rs`: one positive formation and the closest look-alike that must **not** fire.
4. Add the names to `scripts/gen_i18n.py` (`scene.<kind>`) and to `scene_name` in `src/bin/reco-minimap.rs`, in all four languages, and run the script.

Useful building blocks: `profile.landmark("spot_l")`, `profile.zone_rect('l', 1)` (penalty area / key), `profile.field.edge_dist(pos)`, `AgentOut.vel`, and the ball state in the frame.
