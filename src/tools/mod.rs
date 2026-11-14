//! Support code for the `reco-minimap` command-line program (feature `tools`).

pub mod calib;
pub mod async_ocr;
pub mod board_io;
pub mod board_srv;
pub mod colors;
pub mod i18n;
pub mod infer;
pub mod live;
#[cfg(target_os = "macos")]
pub mod mac_app;
pub mod models;
pub mod ocr;
pub mod progress;
pub mod review;
pub mod scales;
pub mod stats;
pub mod ticker_pub;
pub mod ticker_sim;
pub mod ui;
pub mod update;
pub mod video;
