//! Optional two-rate detection. A separately supplied ball model sees native
//! resolution crops at every analysed timestamp; the main model sees people
//! less often. No previous detections are relabelled as fresh observations.
use super::{infer::RfDetr, video::Frame};
use crate::{
    detect::{nms, plan_tiles, Class, Det, Tile},
    search::MotionSearch,
};

pub struct DualDetector {
    pub ball: RfDetr,
    tiles: Vec<Tile>,
    search: MotionSearch,
    last_people: Option<f64>,
    interval: f64,
    recovery_budget: usize,
    pub people_passes: usize,
    pub ball_tiles: usize,
}

pub struct Pass {
    pub detections: Vec<Det>,
    pub people_observed: bool,
    pub person_tiles: usize,
    pub ball_tiles: usize,
}

/// Kept independent of the model so scheduling can be regression-tested.
pub fn people_due(last: Option<f64>, t: f64, interval: f64, recover: bool) -> bool {
    recover || last.is_none_or(|last| t < last || t - last + 1e-9 >= interval)
}

impl DualDetector {
    pub fn new(
        ball: RfDetr,
        width: u32,
        height: u32,
        hz: f64,
        full_secs: f64,
        recovery_budget: usize,
    ) -> Result<Self, String> {
        if !hz.is_finite() || !(1.0..=120.0).contains(&hz) {
            return Err("--person-hz: finite 1..120 required".into());
        }
        if !ball.cfg.class_map.contains(&Some(Class::Ball)) {
            return Err("--ball-model has no mapped ball class".into());
        }
        // The whole image includes high airborne balls, independent of the
        // primary model's ground-court ROI. Native crops preserve tiny details.
        let tiles = plan_tiles(
            Tile {
                x: 0,
                y: 0,
                w: width,
                h: height,
            },
            ball.size as u32,
            0.2,
        );
        Ok(Self {
            ball,
            tiles,
            search: MotionSearch::new(full_secs),
            last_people: None,
            interval: 1.0 / hz,
            recovery_budget,
            people_passes: 0,
            ball_tiles: 0,
        })
    }

    pub fn detect(
        &mut self,
        primary: &mut RfDetr,
        frame: &Frame,
        tiles: &[Tile],
        t: f64,
        recover_people: bool,
    ) -> Result<Pass, String> {
        let people_observed = people_due(self.last_people, t, self.interval, recover_people);
        let mut detections = if people_observed {
            self.last_people = Some(t);
            self.people_passes += 1;
            primary.detect(frame, tiles)?
        } else {
            Vec::new()
        };
        let plan = self.search.select(
            t,
            &self.tiles,
            frame.w as u32,
            frame.h as u32,
            self.recovery_budget,
        );
        let selected: Vec<_> = plan.indices.iter().map(|&i| self.tiles[i]).collect();
        let balls = self
            .ball
            .detect(frame, &selected)?
            .into_iter()
            .filter(|d| d.class == Class::Ball);
        let mut merged: Vec<_> = detections
            .iter()
            .filter(|d| d.class == Class::Ball)
            .cloned()
            .collect();
        merged.extend(balls);
        merged = nms(merged, 0.5);
        // Hand regions seed the next search even while the ball is occluded.
        let mut focus = merged.clone();
        focus.extend(
            detections
                .iter()
                .filter(|d| d.class != Class::Ball)
                .map(|d| {
                    let mut hand = d.clone();
                    hand.cy -= d.h * 0.2;
                    hand.h *= 0.5;
                    hand
                }),
        );
        self.search.observe(t, &focus);
        detections.retain(|d| d.class != Class::Ball);
        detections.extend(merged);
        self.ball_tiles += selected.len();
        Ok(Pass {
            detections,
            people_observed,
            person_tiles: if people_observed { tiles.len() } else { 0 },
            ball_tiles: selected.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::people_due;
    #[test]
    fn ball_updates_need_not_run_the_person_model_and_recovery_is_immediate() {
        assert!(people_due(None, 0.0, 0.1, false));
        assert!(!people_due(Some(0.0), 1.0 / 60.0, 0.1, false));
        assert!(people_due(Some(0.0), 0.1, 0.1, false));
        assert!(people_due(Some(0.0), 0.02, 0.1, true));
        assert!(people_due(Some(1.0), 0.0, 0.1, false));
    }
}
