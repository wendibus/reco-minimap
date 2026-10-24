//! Serves the live ticker with made-up events, to look at the page without a video:
//!   cargo run --features tools --example ticker_demo -- [--port 8765] [--names both] [--sport football] [--approve] [--export dir] [--push url]
//! then open http://127.0.0.1:8765/ticker
use reco_minimap::tools::live::{self, Hub, TickerCfg};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_string());
    let mode = TickerCfg::parse_mode(&arg("--names", "numbers")).unwrap_or("numbers");
    let cfg = TickerCfg {
        names: mode.into(),
        home_name: "Schapen Sharks".into(),
        away_name: "Gäste".into(),
        home_players: reco_minimap::jersey::parse_roster_names("7:Anna Müller, 12=Ben, 4:Cem"),
        away_players: reco_minimap::jersey::parse_roster_names("9:Dana"),
        sport: arg("--sport", "basketball"),
        approve: args.iter().any(|a| a == "--approve"),
    };
    let hub = Hub::with_ticker("{}".into(), Some(cfg));
    let addr = live::serve(hub.clone(), "127.0.0.1", arg("--port", "8765").parse().unwrap()).unwrap();
    eprintln!("http://{addr}/ticker");
    if let Some(url) = args.iter().position(|a| a == "--push").and_then(|i| args.get(i + 1)) {
        let sink = reco_minimap::tools::ticker_pub::Sink { push: Some(url.clone()), method: "PUT".into(), ..Default::default() };
        reco_minimap::tools::ticker_pub::check(&sink).unwrap();
        reco_minimap::tools::ticker_pub::spawn(hub.clone(), sink);
    }
    if let Some(dir) = args.iter().position(|a| a == "--export").and_then(|i| args.get(i + 1)) {
        let sink = live::ticker_sink_dir(dir);
        reco_minimap::tools::ticker_pub::check(&sink).unwrap();
        reco_minimap::tools::ticker_pub::spawn(hub.clone(), sink);
    }
    let script = [
        json!({"type": "scene", "kind": "tip_off", "team": "none", "side": null, "t": 2.0, "borderline": false}),
        json!({"type": "scene", "kind": "shot3", "team": "home", "side": "r", "t": 95.0, "borderline": true, "phase": "attempt"}),
        json!({"type": "sub", "kind": "possible", "team": "away", "out": 9, "in": null, "t": 130.0}),
        json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 12, "t": 181.0}),
        json!({"type": "scene", "kind": "free_throw", "team": "home", "side": "l", "t": 200.0, "phase": "attempt"}),
        json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 4, "in": null, "t": 260.0}),
    ];
    for e in script {
        hub.push_events(&[e]);
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
