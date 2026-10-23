//! Bounded background OCR: at most one batch is outstanding. Only torso crops
//! cross the thread boundary, never a whole panorama or mutable tracker state.
use super::ocr::{self, Crop, NumberReader, Reading};
use crate::{
    team::Team,
    tracker::{Track, TrackState},
};
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    time::Instant,
};

#[derive(Clone, Copy)]
pub struct Identity {
    pub id: u32,
    pub segment: u32,
    pub team: Team,
    pub observed_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geom::Vec2,
        tracker::{BBox, Obs, Tracker, TrackerCfg},
    };
    #[test]
    fn identity_rejects_reused_lost_wrong_team_and_stale_tracks() {
        let mut tracker = Tracker::new(TrackerCfg::default());
        tracker.update(
            1.0,
            &[Obs {
                referee: false,
                team: Team::Home,
                keeper: false,
                pos: Vec2::new(2.0, 2.0),
                bbox: BBox::default(),
                conf: 0.9,
            }],
        );
        let mut tr = tracker.tracks()[0].clone();
        let key = Identity {
            id: tr.id,
            segment: tr.segment,
            team: tr.team,
            observed_at: 1.0,
        };
        assert!(key.matches(&tr, 2.0));
        assert!(!key.matches(&tr, 4.1));
        assert!(!key.matches(&tr, 0.5));
        assert!(!key.matches(&tr, f64::NAN));
        tr.segment += 1;
        assert!(!key.matches(&tr, 2.0));
        tr.segment -= 1;
        tr.team = Team::Away;
        assert!(!key.matches(&tr, 2.0));
        tr.team = key.team;
        tr.state = TrackState::Lost;
        assert!(!key.matches(&tr, 2.0));
    }
    #[test]
    fn slow_worker_does_not_block_submission_or_allow_backlog() {
        let (release, wait) = mpsc::sync_channel(1);
        let mut worker = BackgroundOcr::with_reader(
            move |_| {
                wait.recv().unwrap();
                Ok(Reading {
                    probs: vec![],
                    steps: 0,
                })
            },
            0.0,
            4,
        );
        let job = || Job {
            identity: Identity {
                id: 1,
                segment: 0,
                team: Team::Home,
                observed_at: 0.0,
            },
            crop: Crop {
                w: 8,
                h: 8,
                rgb: vec![0; 8 * 8 * 3],
            },
        };
        assert!(worker.submit(vec![job()]));
        assert!(!worker.ready());
        assert!(!worker.submit(vec![job()]));
        assert!(worker.poll().is_none());
        release.send(()).unwrap();
        let result = worker
            .results
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(result.readings.len(), 1);
        assert_eq!(result.errors, 0);
    }
}
impl Identity {
    /// A reused slot is a different observation even when its team is unchanged.
    pub fn matches(&self, tr: &Track, now: f64) -> bool {
        self.id == tr.id
            && self.segment == tr.segment
            && self.team == tr.team
            && tr.state != TrackState::Lost
            && tr.last_seen >= self.observed_at
            && now.is_finite()
            && self.observed_at.is_finite()
            && (0.0..=3.0).contains(&(now - self.observed_at))
    }
}
pub struct Job {
    pub identity: Identity,
    pub crop: Crop,
}
pub struct ResultBatch {
    pub readings: Vec<(Identity, Reading)>,
    pub errors: usize,
}

pub struct BackgroundOcr {
    jobs: SyncSender<Vec<Job>>,
    results: Receiver<ResultBatch>,
    pending: bool,
    pub min_h: f32,
    pub max_reads: usize,
}
impl BackgroundOcr {
    pub fn new(mut reader: NumberReader, min_h: f32, max_reads: usize) -> Self {
        Self::with_reader(move |crop| reader.read(crop), min_h, max_reads)
    }
    fn with_reader(
        mut read: impl FnMut(&Crop) -> Result<Reading, String> + Send + 'static,
        min_h: f32,
        max_reads: usize,
    ) -> Self {
        let (jobs, rx) = mpsc::sync_channel::<Vec<Job>>(1);
        let (tx, results) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(batch) = rx.recv() {
                let started = Instant::now();
                let mut result = ResultBatch {
                    readings: Vec::new(),
                    errors: 0,
                };
                for job in batch {
                    // Cannot cancel an ONNX call, but do not start further reads
                    // when this batch has already exhausted its time budget.
                    if started.elapsed().as_secs_f64() > 2.0 {
                        break;
                    }
                    let crop = ocr::localize(job.crop.w, job.crop.h, &job.crop.rgb);
                    match read(&crop) {
                        Ok(reading) => result.readings.push((job.identity, reading)),
                        Err(_) => result.errors += 1,
                    }
                }
                if tx.send(result).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs,
            results,
            pending: false,
            min_h,
            max_reads: max_reads.min(16),
        }
    }
    pub fn ready(&self) -> bool {
        !self.pending
    }
    pub fn submit(&mut self, jobs: Vec<Job>) -> bool {
        if self.pending || jobs.is_empty() {
            return false;
        }
        self.pending = self.jobs.try_send(jobs).is_ok();
        self.pending
    }
    pub fn poll(&mut self) -> Option<ResultBatch> {
        match self.results.try_recv() {
            Ok(result) => {
                self.pending = false;
                Some(result)
            }
            Err(_) => None,
        }
    }
}
