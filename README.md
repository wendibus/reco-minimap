# reco-minimap

Top-down minimap (simplified court, 2 black referee discs, 5+5 players in jersey
colours, orange ring on the ball holder, small orange dot for a free ball) from
a Reco panorama - live feed or video. Pure Rust, no dependencies.

License: AGPL-3.0-only (same as the reco workspace).

## Programme (`reco-minimap`, Feature `tools`)

Bauen (macOS):

    brew install ffmpeg onnxruntime
    export ORT_DYLIB_PATH=/opt/homebrew/lib/libonnxruntime.dylib   # ONNX Runtime >= 1.24 (wie reco-detect)
    cargo build --release --features tools            # optional: --features tools,coreml
    ./target/release/reco-minimap --help

Ablauf pro Halle/Spiel:

    reco-minimap calibrate  pano.mp4                       # Court-Punkte im Browser anklicken -> court.json
    reco-minimap selfcheck  pano.mp4 --model <modellordner> --annotate check/   # Annahmen pruefen (Text-Report)
    reco-minimap colors     pano.mp4 --model <modellordner> --court court.json  # Trikotfarben -> teams.json
    reco-minimap run        pano.mp4 --model <modellordner> --court court.json --teams teams.json \
                            --pip pip.mp4 --minimap mini.mp4 --alpha overlay.mov --json states.jsonl

* `<video>` = Datei oder jede ffmpeg-Quelle (rtmp/srt/udp/Pipe) -> Live-Panorama: `--size 4096x1500 --fps 25`.
* `--pip` zeigt das Panorama mit Erkennungsboxen und der Minimap im Eck (zum Pruefen).
* `--alpha` = ProRes 4444 mit Transparenz fuer OBS; `--json` = Zustand pro Bild fuer Spieldock.
* Bilder/Videos bleiben lokal; die Kalibrierseite laeuft nur auf 127.0.0.1.
* `scripts/make_test_assets.py` erzeugt ein synthetisches Panorama + Platzhalter-ONNX fuer einen Rauchtest.

## Status

Tested here: `cargo test` (9 tests) und ein Ende-zu-Ende-Lauf aller vier Programme mit
synthetischem Panorama-Video, echtem ONNX Runtime 1.24 und einem Platzhalter-ONNX mit der
Signatur des Reco-Modells (`input [1,3,512,512]` -> `dets [1,300,4]`, `labels [1,300,4]`).
Unit-Tests (Rust 1.75 / 1.91): homography, RF-DETR decoding,
tiling, NMS, Hungarian assignment, 5+5+2 slot stability, occlusion coasting
without teleporting, re-acquisition, ball Held -> hidden -> InFlight, renderer.

NOT tested (needs your machine / real data):
- the real RF-DETR model and real panorama footage (accuracy, speed, class order),
- the browser page itself (server + JS syntax tested, not clicked in a browser),
- live inputs (rtmp/srt/...), CoreML execution (compiles with `--features coreml`),
- the real Reco panorama projection (tests use a planar homography),
- `reco-core` / `reco-detect` integration (not compiled against the workspace).

## Pipeline

    panorama frame -> tiles (plan_tiles) -> RF-DETR 512x512 (reco-detect, ORT)
      -> decode_rfdetr -> tile_to_pano -> nms
      -> Minimap::process(t, &dets) -> FrameState -> Renderer / JSON

## Integration into the reco workspace

1. Copy to `crates/reco-minimap`, add to `[workspace] members`.
2. In `Cargo.toml` use `edition.workspace = true`, `rust-version.workspace = true`,
   `license.workspace = true` (code avoids let-chains, builds on 2021 and 2024).
3. `reco-detect` only decodes YOLO `[1,N,6]`. For RF-DETR add an ORT session
   (input `input` `[1,3,512,512]`, outputs `dets [1,300,4]`, `labels [1,300,4]`),
   call `preprocess_imagenet`, then `decode_rfdetr(dets, labels, &DecodeCfg::basketball_v1())`.
4. Implement `geom::Projector` on top of reco-core's panorama projection
   (undo panorama projection, then a court homography from 4-6 clicked points).
   `HomographyProjector` is the planar fallback.

## Assumptions to verify on a real frame (first thing to do)

- logit order = `runtime.json` classes `[ball, player, referee, ignore]`
  (change `DecodeCfg::class_map` if shifted),
- boxes are normalized `cxcywh` (else `BoxFormat::Xyxy`),
- preprocessing = ImageNet mean/std, RGB, CHW,
- ball threshold 0.30 (model default 0.7 is too high for a tiny ball).

## Known limits

- A flying ball is projected onto the floor plane, so its dot is displaced.
  Offline refinement (parabola between release and catch) is a later step.
- Team colour needs `Det::color` (mean torso colour) sampled by the caller.
- Jersey OCR is not included; `add_jersey_reading` + voting is ready.
