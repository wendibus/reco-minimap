# Tactics-board scene (format 1)

`reco-minimap snapshot` cuts one moment out of a game and writes it as a **scene file**: plain JSON that a tactics-board app (web, iPad, Android) can open, let a coach move players around in, and save again. `reco-minimap svg` draws such a file as a vector graphic.

```
reco-minimap run game.mp4 … --json game.jsonl                  # once per game
reco-minimap snapshot game.jsonl --court court.json --teams teams.json \
    --at 12:34 --trail 5 --out scene.json --svg scene.svg
reco-minimap svg scene.json --draw-home "#ffffff" --no-trails --out scene2.svg   # redraw, e.g. after editing
```

`--at` is video time (seconds, `min:s` or `h:min:s`); the nearest recorded frame is used. `--trail` is how many seconds of movement before that moment travel along (default 5, `0` = none).

## The file

```json
{
  "format": "reco-minimap-board",
  "version": 1,
  "sport": "football",
  "t": 754.2,
  "field": { "length": 105, "width": 68 },
  "teams": {
    "home": { "color": "#ffffff", "keeper_color": "#1ea03c" },
    "away": { "color": "#2f6bff", "keeper_color": null }
  },
  "players": [
    { "id": 3, "team": "home", "number": 7, "keeper": false, "x": 41.2, "y": 30.5,
      "trail": [[38.9, 30.1], [40.0, 30.3]] }
  ],
  "referees": [ { "x": 52.0, "y": 34.0 } ],
  "ball": { "state": "held", "holder": 3, "x": 41.2, "y": 30.5 },
  "lines": [ [[0, 0], [105, 0], [105, 68], [0, 68], [0, 0]] ],
  "goals": [ [0, 34], [105, 34] ]
}
```

* All positions are **metres**, `x` along the field (left to right), `y` across (far to near sideline), origin at the top-left corner of the field.
* `team` is `home` or `away`. `number` is the confirmed jersey number or `null`. `keeper` is `true` for a goalkeeper (only in sports with a keeper kit: football, futsal, handball, hockey, lacrosse).
* `trail` lists earlier positions of the same player, oldest first, ending just before `x`/`y`.
* `ball.state` is `held`, `flight`, `loose` or `unknown`; `holder` is a player `id` when held. `x`/`y` are `null` when the ball is not known.
* `lines` are the field markings as polylines, so a board needs no sport tables of its own. `goals` are goal or basket centres.
* `teams.*.color` is the **drawing** colour (`--draw-home` / `--draw-away`, otherwise the detected shirt colour). `keeper_color` is the stripe colour of that team's keeper, or `null` for black/white by contrast.
* Readers must check `format` and refuse a larger `version`; unknown extra keys are to be kept when saving so older programs do not lose data.

A board that lets the coach move players should write the same structure back (change `x`, `y`, `trail`, `ball`); `reco-minimap svg` then draws it.

## The SVG

The SVG's `viewBox` is in metres (field plus a 1.5 m margin), so it scales freely. It is meant to be used by a web board directly:

* `<g id="court">` – field lines, `<g id="trails">` – movement trails, `<g id="referees">`, `<g id="players">`, `<circle id="ball">`.
* Each player is `<g class="player home|away [keeper]" data-id="3" data-number="7" transform="translate(x y)">` – move it by changing the transform. The ball holder carries a ring (`class="holder"`), keepers are striped (`url(#stripes-home)` / `url(#stripes-away)`), the root carries `data-format`, `data-version`, `data-sport`, `data-t`.

Options: `--width px` (default 900, only the preset size), `--no-numbers`, `--no-trails`, `--no-stripes`, `--draw-home`, `--draw-away`, `--draw-home-gk`, `--draw-away-gk`.
