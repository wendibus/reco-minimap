#!/usr/bin/env python3
"""Evaluate sparse, manually labelled frames. Unlabelled fields are not scored.

This reports explicit operational metrics, not HOTA/MOTA or calibrated confidence.
Coordinates must use the same calibrated court plane; airborne ball positions
must not be compared to a physical ground-truth position on the floor.
"""
import argparse
from bisect import bisect_left
import json
import math
from pathlib import Path


def read_frames(path):
    frames = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    times = [f["t"] for f in frames]
    if any(not math.isfinite(t) for t in times) or any(a >= b for a, b in zip(times, times[1:])):
        raise ValueError("frame timestamps must be finite and strictly increasing")
    return frames


def assignment(cost):
    """Minimum-cost rectangular assignment, rows <= columns; dummy columns allow misses."""
    if not cost:
        return []
    n, m = len(cost), len(cost[0])
    u, v, p, way = [0.]*(n+1), [0.]*(m+1), [0]*(m+1), [0]*(m+1)
    for row in range(1, n+1):
        p[0] = row
        j0 = 0
        best, used = [math.inf]*(m+1), [False]*(m+1)
        while True:
            used[j0] = True
            i0, delta, j1 = p[j0], math.inf, 0
            for j in range(1, m+1):
                if not used[j]:
                    value = cost[i0-1][j-1]-u[i0]-v[j]
                    if value < best[j]:
                        best[j], way[j] = value, j0
                    if best[j] < delta:
                        delta, j1 = best[j], j
            for j in range(m+1):
                if used[j]:
                    u[p[j]] += delta
                    v[j] -= delta
                else:
                    best[j] -= delta
            j0 = j1
            if p[j0] == 0:
                break
        while j0:
            j1 = way[j0]
            p[j0] = p[j1]
            j0 = j1
    result = [None]*n
    for j in range(1, m+1):
        if p[j]:
            result[p[j]-1] = j-1
    return result


def xy(p):
    return all(isinstance(p.get(k), (int, float)) and math.isfinite(p[k]) for k in ("x", "y"))


def event_metrics(predictions, reference, tolerance=1.0):
    # False positives can only be scored inside explicitly annotated windows.
    windows=[]
    targets=[]
    for frame in reference:
        if "event_window" in frame:
            start,end=frame["event_window"]
            if not math.isfinite(start) or not math.isfinite(end) or end<start:
                raise ValueError("invalid event_window")
            windows.append((start,end))
        for event in frame.get("events",[]):
            targets.append({"t":frame["t"],**event})
    if not windows:
        return {"event_targets":None,"event_precision":None,"event_recall":None}
    inside=lambda t:any(start<=t<=end for start,end in windows)
    if any(not inside(e["t"]) or "kind" not in e for e in targets):
        raise ValueError("labelled events need a kind and must lie in an event_window")
    predicted=[]
    seen=set()
    for frame in predictions:
        for event in frame.get("events",[]):
            item={"t":frame["t"],**event}
            key=tuple(item.get(k) for k in ("t","type","kind","phase","team"))
            if inside(item["t"]) and key not in seen:
                seen.add(key);predicted.append(item)
    costs=[[abs(g["t"]-p["t"]) if all(g.get(k)==p.get(k) for k in ("kind","type","phase","team") if k in g) else 1e6 for p in predicted]
           + [tolerance+1e-6]*len(targets) for g in targets]
    chosen=assignment(costs)
    hits=sum(col<len(predicted) and costs[row][col]<=tolerance for row,col in enumerate(chosen))
    return {"event_targets":len(targets),"predicted_events_in_labelled_windows":len(predicted),
            "correct_events":hits,"false_events":len(predicted)-hits,
            "event_precision":hits/len(predicted) if predicted else None,
            "event_recall":hits/len(targets) if targets else None}


def evaluate(predictions, reference, time_tolerance=0.02, distance_gate=2.0):
    times = [f["t"] for f in predictions]
    counts = dict(labelled_frames=len(reference), matched_frames=0, player_targets=0,
                  player_matches=0, extra_players=0, identity_targets=0, correct_identities=0,
                  sampled_id_switches=0, ball_targets=0, ball_matches=0, false_ball_positions=0,
                  ball_state_targets=0, correct_ball_states=0, possession_targets=0, correct_possessions=0)
    player_errors, ball_errors, previous = [], [], {}
    for gt in reference:
        at = bisect_left(times, gt["t"])
        options = [i for i in (at-1, at) if 0 <= i < len(times)]
        idx = min(options, key=lambda i: abs(times[i]-gt["t"])) if options else None
        matched = idx is not None and abs(times[idx]-gt["t"]) <= time_tolerance
        frame = predictions[idx] if matched else {"players": [], "ball": {}}
        counts["matched_frames"] += matched
        mapping = {}
        if "players" in gt:
            targets = gt["players"]
            if any(not xy(p) for p in targets):
                raise ValueError("labelled players require finite x/y positions")
            # Coasting positions count as predictions, but are not detections.
            players = [p for p in frame.get("players", []) if p.get("state") != "lost" and xy(p)]
            costs = [[math.dist((g["x"],g["y"]),(p["x"],p["y"]))
                      if g.get("team",p.get("team")) == p.get("team") else 1e6 for p in players]
                     + [distance_gate]*len(targets) for g in targets]
            chosen = assignment(costs)
            used = set()
            counts["player_targets"] += len(targets)
            for row, (g, col) in enumerate(zip(targets, chosen)):
                if g.get("number") is not None:
                    counts["identity_targets"] += 1
                if col >= len(players) or costs[row][col] >= distance_gate:
                    continue
                p = players[col]
                used.add(col)
                counts["player_matches"] += 1
                player_errors.append(costs[row][col])
                if g.get("number") is not None and p.get("jersey") == g["number"]:
                    counts["correct_identities"] += 1
                if "id" in g:
                    key = (p["id"],p.get("segment",0))
                    mapping[p["id"]] = g["id"]
                    old = previous.get(g["id"])
                    if old is not None and old[0] != key and gt["t"]-old[1] <= 1.0:
                        counts["sampled_id_switches"] += 1
                    previous[g["id"]] = (key,gt["t"])
            counts["extra_players"] += len(players)-len(used)
        ball = gt.get("ball", {})
        predicted = frame.get("ball", {})
        if xy(ball):
            counts["ball_targets"] += 1
            if xy(predicted):
                error = math.dist((ball["x"],ball["y"]),(predicted["x"],predicted["y"]))
                ball_errors.append(error)
                counts["ball_matches"] += error < distance_gate
        elif ball.get("present") is False and xy(predicted):
            counts["false_ball_positions"] += 1
        if "state" in ball:
            counts["ball_state_targets"] += 1
            counts["correct_ball_states"] += predicted.get("state") == ball["state"]
        if "holder_player_id" in ball:
            counts["possession_targets"] += 1
            counts["correct_possessions"] += predicted.get("state") == "held" and mapping.get(predicted.get("holder")) == ball["holder_player_id"]
    ratio = lambda a,b: counts[a]/counts[b] if counts[b] else None
    return {**counts, **event_metrics(predictions,reference), "player_recall":ratio("player_matches","player_targets"),
            "identity_accuracy":ratio("correct_identities","identity_targets"),
            "ball_position_recall":ratio("ball_matches","ball_targets"),
            "ball_state_accuracy":ratio("correct_ball_states","ball_state_targets"),
            "possession_accuracy":ratio("correct_possessions","possession_targets"),
            "player_matched_mean_error_m":sum(player_errors)/len(player_errors) if player_errors else None,
            "ball_mean_error_m":sum(ball_errors)/len(ball_errors) if ball_errors else None,
            "time_tolerance_s":time_tolerance,"distance_gate_m":distance_gate}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--time-tolerance", type=float, default=0.02)
    parser.add_argument("--distance-gate", type=float, default=2.0)
    args = parser.parse_args()
    if not math.isfinite(args.time_tolerance) or args.time_tolerance < 0 or not math.isfinite(args.distance_gate) or args.distance_gate <= 0:
        parser.error("finite nonnegative time tolerance and positive distance gate required")
    reference = read_frames(args.reference)
    if not reference:
        parser.error("reference must contain annotated frames")
    report = {mode:evaluate(read_frames(path),reference,args.time_tolerance,args.distance_gate)
              for mode,path in (("baseline",args.baseline),("candidate",args.candidate))}
    with args.out.open("x") as f:
        json.dump(report,f,indent=2)
        f.write("\n")
    print(json.dumps(report,indent=2))


if __name__ == "__main__":
    main()
