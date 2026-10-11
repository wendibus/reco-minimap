# Walkthrough: from two camera videos to a minimap

This guide takes you from "I have a panorama video and a trained model" to "I have a minimap video". It follows the six steps of the interface. Allow about 30 minutes the first time; later games take a few minutes of clicking plus processing time.

**What you need before you start**

* The reco-minimap program installed, see [Installation](#installation) below.
* A **panorama video of the whole field** – the stitched picture from Reco, not the AI-cropped view. If you only have a left and a right camera video, stitch them with Reco first. A 1920×1080 export is enough for the minimap; for jersey numbers you want a larger one (see step 4).
* A **model exported by Reco Trainer** (a folder with `runtime.json`, `model-card.json` and an `.onnx` file).
* Optional: the shirt numbers of both squads, if you want numbers on the dots.

Start the interface:

```bash
reco-minimap ui
```

Your browser opens at `http://127.0.0.1:8788/`. Pick your language at the top right (Deutsch · English · Français · Español); the program's messages in the log follow that choice. At the top you also see the **project folder** – everything for one venue or season is stored there. The default is `~/reco-minimap-projekte/standard`; use *Browse …* to choose another folder.

> Tip: use one project folder per venue and camera position. The calibration depends on where the camera stands.

## Installation

| | |
|---|---|
| Rust | stable toolchain (built and tested with **1.91**) |
| ffmpeg + ffprobe | on your `PATH` (`brew install ffmpeg`) |
| ONNX Runtime | shared library **1.24 or newer** |
| A detection model | an RF-DETR ONNX export, e.g. from Reco Trainer |

**macOS**

```bash
brew install ffmpeg
python3 -m venv ~/ortvenv
~/ortvenv/bin/pip install onnxruntime
find ~/ortvenv -name "libonnxruntime*.dylib"
export ORT_DYLIB_PATH="<the path shown>"
cargo install --path . --features tools,coreml
```

The library from the Python package includes CoreML; the Homebrew one does not. Put the `export ORT_DYLIB_PATH=…` line into `~/.zshrc` so it holds in new terminal windows.

**Linux**

```bash
sudo apt install ffmpeg
python3 -m venv ~/ortvenv && ~/ortvenv/bin/pip install onnxruntime
export ORT_DYLIB_PATH=$(find ~/ortvenv -name "libonnxruntime.so*" | head -1)
cargo install --path . --features tools
```

Tested on macOS (Apple silicon); the test suite runs on Linux; Windows is untested.

## How it is meant to be used

The work has two parts: **set up once**, then **process each game**.

**Once per venue and camera position** (steps 1 to 3)

1. Pick the sport and import the model.
2. Choose a panorama video from that venue.
3. Calibrate: click the field landmarks, measure player sizes, set the shirt colours.

The result lives in the project folder (`court.json`, `scales.json`, `teams.json`) and stays valid as long as the camera stands in the same place and the panorama is exported the same way. If either changes, repeat step 3.

**For each game** (steps 2, 3c, 4 and 6)

1. Choose the new video (step 2).
2. Detect the shirt colours again, or set them by hand, when other teams play (step 3c).
3. Do a **short trial run** of 20 to 60 seconds with the check video and watch it (step 4).
4. If it looks right, process the whole game in segments.
5. Look at the numbers in step 6 if something seems odd.

**What the program decides and what you provide**

* It finds players, referees and the ball, tracks them and keeps a fixed number of slots per team. A hidden player stays at his estimated position instead of disappearing.
* You provide where the field is, which colours the teams wear and which time span to process. These three decide the quality; everything else has usable defaults.
* The check video (*Panorama with boxes and minimap*) is your inspection tool. Watch it for the first segment before you let a whole game run.

**At the bottom of the page** is the log. It shows every command the interface runs and its output. Everything the interface does also works on the command line; a few fine settings exist only there (`reco-minimap --help`).

**New versions** are announced at the top with a blue notice, see [Updating](#updating).

---

## Step 1 · Sport & model

1. **Choose the sport.** The list matches Reco Trainer. The grey line below shows field size, players per team and referee slots. For a sport that is not listed, choose *Other sport (rectangular field)* and enter length, width, players per team and referees.
2. **Import the model.** Click *Choose folder and import …* and select the folder from Reco Trainer (or *Choose .onnx file and import …* for a bare file). You can also type or paste a path and press *Import path*.
   * The program checks the checksum of the weights. *checksum OK* is what you want; a damaged file is refused.
   * It shows the classes (`ball, player, referee, ignore`), the input size and the sport the model was trained for. If the model names a sport, that sport is selected for you. If it differs from your choice, a red note tells you.
3. **Check the model** (needs a video from step 2, so you can come back to this): *Check model on the video* runs the model on a few frames and writes a report in the log at the bottom. What to look for:
   * *Players per frame (median)* close to the number on the field (10 for basketball). Much lower means the model misses players at this size – continue anyway, step 3 improves it.
   * `Wandzeit pro Bild` is the time per frame. On an M4 Pro with CoreML expect a few hundred milliseconds.
   * It tells you if the model's assumptions about its outputs are met ("boxes in 0..1", "most hits on `player`").

## Step 2 · Video

Choose the **panorama video** (*Browse …* or paste a path). The program reads size, frame rate and length and shows them in green, e.g. `1920×1080, 25 fps, 20:00 min`. A red message means the file could not be read – check that `ffmpeg` is installed.

## Step 3 · Calibration

Calibration teaches the program where the field is in the picture. Three things happen here, in this order.

### 3a · Calibrate the field

1. Set *Moment in the video* to a time where the **whole field is visible** and not blocked by people (for example during a timeout or warm-up).
2. Click *Start calibration*. A page opens in your browser showing that frame.
3. **Click the landmarks in the order listed on the right** (corners, halfway line, centre, penalty/key corners …). The list is in your language.
   * Landmark names use *far* and *near* for the two sidelines (far = away from the camera) and *left/right* as seen in the picture.
   * You do not have to click everything. Skip points you cannot see with the **space bar**.
   * Above the list on the right, a **small top-down field** shows where the landmark being asked for lies: it **blinks orange**. Landmarks already set are green, open ones grey. In this drawing the camera is at the bottom and *far* is at the top.
   * Mouse wheel = zoom, drag = move, **Backspace** = remove the last point, **F** = show the whole picture. The small window top right magnifies the area under the cursor.
4. After the 4th point a **green field drawing** appears over the picture and the page shows the **error in metres**. Add points until the green lines sit on the real lines. Aim for **8 or more points spread over the whole field** and an error **below 0.5 m** (green). More points on different sides matter more than clicking precisely.
5. Click *Save*. Back in the interface the line reads e.g. `15 points, error 0.21 m`.

> An error of 0.00 m after exactly four points means nothing – four points always fit perfectly. Judge the green lines and use more points.

If your venue is not standard size, click only the four corners (and the centre); the other landmarks assume a standard field.

### 3b · Measure player sizes

Click *Measure*. The program looks at a dozen frames and learns how tall a player appears at each image row. Later, each band of the picture is searched with the tile size that suits it (`--tile auto`), which makes detection both better and about twice as fast. It needs the calibration from 3a. Run it once per camera position.

### 3c · Shirt colours

1. Click *Detect colours*. The program samples the shirts of the players on the field and splits them into two teams. The log shows the colour groups it found.
2. Set the colours:
   * **Home / Away (detection)** – the colours as the camera sees them. These are filled in automatically; use *Swap* if home and away are the wrong way round. If it fails ("teams not distinguishable"), pick the two colours by hand with the colour pickers.
   * **Home / Away (display)** – the colours used to draw the dots. Choose what looks right, for example white and a bright blue. A very dark display colour is hard to see on the dark minimap.
   * **Referee colour** – the shirt colour of the referees (usually black or yellow). This lets the program find referees even if the model does not.
3. **Figures and shirts** (change only when needed):
   * **Shirt area, top / bottom** – which part of a player box shows the shirt, as a share of the box height from the top. The default 0.18 to 0.5 fits people. Adjust it for figures with other proportions (mascots, animations), then run *Detect colours* again.
   * **Match the colour sample to the shirt colours** – helps with strong, clearly different colours (red against blue) and when areas of the court are painted in a team colour. Leave it off for light against dark: there it assigns more players to the wrong team.
   * **Game without referees** – nobody is then taken for a referee because of an odd shirt colour.

## Step 4 · Process

1. **Segments.** Each row is one run: a *Name*, a *Start* and an *End* as `min:s` (or `h:min:s`, or plain seconds). An empty end means up to the end of the video, so you can analyse just a part of a long recording. Splitting a game into quarters is safer – an interruption costs one quarter, not the whole game. *Add segment* adds a row.
2. **Outputs** (tick what you want):
   * *Minimap video* – the minimap on a dark background.
   * *Overlay with transparency (.mov)* – the minimap with a transparent background, to place over a video in an editor or OBS. *Dark backing in the overlay* (0–1) adds a semi-transparent dark box so it stays readable on a bright picture; 0.6 is a good start.
   * *Panorama with boxes and minimap* – your panorama with detection boxes and the minimap in the corner. Use this to **check quality** on your first segment.
   * *Data (.jsonl)* – one record per frame with all positions ([format](../README.md#output-formats)).
3. **Detect on every Nth frame.** 5 is a good default: detection runs on every 5th frame and the minimap video is interpolated to the full frame rate, so it still looks smooth. Higher is faster, lower is more exact.
4. **Opacity of undetected players.** 1 keeps a hidden player fully visible at his estimated position; lower values fade him.
5. **Compute.** *automatic* picks CoreML on a Mac. Leave *CoreML* on `all`.
6. Click **Start**. The log at the bottom shows progress and, at the end, a summary: frames processed, detections, time per stage. *Cancel* stops the run.

**Where are my files?** In `<project folder>/out/`, named after the segment (`segment1.mp4`, `segment1.mov`, `segment1_pip.mp4`, `segment1.jsonl`). The list below the settings shows the newest files with *Show in folder*.

**Do a quick first run.** Use a segment of 20–60 seconds with *Panorama with boxes and minimap* ticked. Open `…_pip.mp4` and check:

* Do the dots sit on the right people and in the right places on the field? If not, the calibration needs more or better-spread points (step 3a).
* Are there phantom dots or dots that jump? A few jumps are normal; many point to a poor calibration or colours that are too similar.
* Are the referees marked? If not, check the referee colour (step 3c).

Then process the whole game.

### Jersey numbers (optional)

In step 4 there is a section *Read and show jersey numbers*.

1. Click *Download model* once (about 11 MB). The status turns to "Number model installed".
2. Enter the **squad numbers** of both teams, separated by commas (`4,7,10,12,23`). The reader then only chooses among these – much more reliable.
3. Tick *Read and show jersey numbers* and run a segment.
4. **Size matters.** A number is readable from roughly 10 px digit height, which means players about **80 px tall**. At the end of the run the log says how tall players are in your video and whether that is enough. If not, export a larger version of the same panorama with Reco (same timeline, same frame rate) and enter it as *Higher-resolution panorama for the numbers*. Detection keeps using the smaller video.
5. Numbers appear on the dots as soon as they are confirmed. Players whose back is never visible stay without a number.

**Substitutions** show up in the log (`SUBSTITUTION home at 12:34: #7 -> #12` for a confirmed change, "possible substitution" when someone left and someone entered at the sideline but numbers are unknown), in `events` of the data file and in step 6.

### Game situations (optional)

Tick *Detect game situations* (in step 4, also available in step 5). The program then recognises the **formations at stoppages** and, in basketball, **shot attempts**:

* Basketball: tip-off, free throw, throw-in (inbound), 2- and 3-point attempts, substitutions.
* Football: kick-off, penalty, corner, throw-in, substitutions.
* Handball: throw-off, 7 m throw, throw-in, substitutions.
* Futsal, hockey, rugby, lacrosse: see the [catalogue per sport](SCENARIOS.md).

What to expect:

* A situation is reported when the formation has held for a moment – for example a player at the free-throw line, three players in the lane, everybody else standing still. The log then says e.g. `SITUATION free throw left (home) at 12:34 (confidence 0.85)`.
* **Check it on a short segment first** and compare with the video. Thresholds are first estimates; a throw-in can be reported when a player simply stands at the line, and a situation is missed when many players are not detected.
* **2- and 3-point attempts** need the ball to be found when it leaves the shooter's hands. They tell you the type of attempt, not whether it scored. A "close to the line" note means the position error is as large as the distance to the 3-point line.
* The scenes appear in the data file as events with `"type": "scene"` and in step 6.

## Step 5 · Live (optional)

The live mode serves a transparent overlay page for OBS while the video plays. It is meant for streams; for post-production use the `.mov` overlay from step 4.

1. *Source*: a file, or a stream address (`srt://…`, `rtmp://…`, `udp://…`). For a first test choose a video file and tick *Play file in real time*.
2. Enter *Size* (for example `1920x1080`) and *Frames/s*. Keep the port (8765) unless it is taken.
3. Click *Start live*. The page shows the overlay address, e.g. `http://127.0.0.1:8765/`.
4. **In OBS:** add a *Browser* source, paste the address, set width 1400 and height 800. The background is transparent.
5. Open the address with `?floor=1&debug=1` in a normal browser to test without OBS (dark floor, status line).
6. *Stop* ends it.

The live mode needs the same full panorama you calibrated on, not the AI-cropped view.

## Step 6 · Evaluation

Choose a `.jsonl` file from `out/` and click *Evaluate*. The report in the log (its lines are in German) answers:

| Line (German in the log) | Good looks like |
|---|---|
| exactly 5+5 players in …% of frames (`genau 5+5 in …%`, basketball) | above 70 % |
| different player IDs per team (`Verschiedene Spieler-IDs`) | at most about 8 per team (more means slots are reassigned often) |
| share of players visible (`Sichtbar`) | above 85 % |
| steps at the speed limit (`Schritte an der Tempogrenze`) | under 3 % |
| 2 referees in …% of frames (`2 Schiris in …%`) | high, if referees are on the field |
| ball states (`Ball: gehalten / Flug / frei / unbekannt`) | the share of *unknown* shows how often the ball was not found; with a weak ball model it can be high |
| substitutions (`Wechsel`) | the list of changes with time and numbers |
| game situations (`Situationen`) | each detected situation with time, team and confidence – compare a few with the video |

---

## Updating

When it starts, the interface asks GitHub once whether a newer version exists. Only the request itself is sent, no video, frame or project data.

* If there is one, a **blue notice** appears at the top with the version number, a *What's new?* link and the button *Install now*.
* *Install now* fetches the source of that version and rebuilds the program. This takes a few minutes and needs the same Rust installation you used to install the program. The log shows the progress.
* Afterwards stop the program in the terminal with **Ctrl+C** and start it again.

On the command line: `reco-minimap update` tells you whether there is something new; `reco-minimap update --yes` installs it. Switch the check at start-up off with `reco-minimap ui --no-update-check`.

## Using the results

**Put the minimap into a game video with ffmpeg** (overlay `segment1.mov` onto `game.mp4`, bottom right, 480 px wide):

```bash
ffmpeg -i game.mp4 -i out/segment1.mov \
  -filter_complex "[1:v]scale=480:-1[m];[0:v][m]overlay=W-w-30:H-h-30:format=auto" \
  -c:v libx264 -crf 18 -pix_fmt yuv420p -c:a copy game_with_minimap.mp4
```

The overlay starts at the segment's start time; use `-ss` on the game video if your segment did not begin at zero. For whole games in several segments, `scripts/run_game.sh` does the processing and the compositing for you.

**Use the data** (`.jsonl`) in your own analysis; the format is in the [README](../README.md#output-formats).

---

## When something goes wrong

| Message or symptom | What to do |
|---|---|
| *ONNX Runtime library not found* / red banner at the top | Set `ORT_DYLIB_PATH` to the `libonnxruntime…dylib` (version 1.24 or newer) and start `reco-minimap ui` from that same terminal. See [Install](../README.md#install). |
| *CoreML provider is not included* | Use the library from the Python wheel (`pip install onnxruntime`), not the Homebrew one – or set *Compute* to `CPU`. |
| *ffmpeg not found* | `brew install ffmpeg` |
| Nothing happens on *Browse …* | The file dialogs need macOS (or `zenity` on Linux). Paste the path instead. |
| `zsh: command not found: #…` | Do not paste lines starting with `#` into the terminal – zsh does not treat them as comments. |
| *Detect colours* fails | Check the calibration (only players inside the calibrated field are counted) and set the colours by hand. |
| Few players detected | Run *Measure* (3b). The program already follows a player on uncertain detections (from 0.30) but only starts a dot on a confident one (from 0.60); if the model is uncertain throughout on your footage, lower `--thr-player` on the command line (e.g. 0.5). A small or blurry picture limits the model. |
| Team colours wrong, or dots in the wrong colour | Set the colours by hand in 3c. For strong colours or coloured court areas switch on *Match the colour sample to the shirt colours*. |
| Two "referees" although there are none | Switch on *Game without referees* in 3c. |
| Dots are in the wrong place | Calibrate again with more points spread over the whole field; the green field drawing must sit on the real lines. |
| Referees missing | Set the referee shirt colour (3c). |
| Very slow | Use *Detect on every Nth frame* with a higher N; make sure *Compute* is CoreML; run *Measure* so that fewer tiles are needed. |
| Numbers stay empty | Is the number model installed? Are players tall enough (see the summary at the end of a run)? Did you enter the squad lists? |
| The port is taken | Start with `reco-minimap ui --port 8800`. |

Still stuck? Open the log (bottom of the page) – it shows the exact command that was run (`$ reco-minimap …`). You can paste it into a terminal to see the same output with more detail.
