//! Arrival path for a programmed pointer point.
//!
//! The whole segment is played. A click, move, or scroll does not warp ahead
//! of the motion. Smoothstep keeps the first and last frames short so the
//! pointer leaves where it is and settles on the exact target. Duration tracks
//! distance and stays inside 64–240 ms, one sample every 16 ms.
use serde::Serialize;

pub const STEP_MS: u64 = 16;
const MIN_PX: f64 = 8.0;
const MIN_MS: f64 = 64.0;
const MAX_MS: f64 = 240.0;
const MS_PER_PX: f64 = 0.18;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Glide {
    pub steps: Vec<[f64; 2]>,
}

pub fn samples(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Glide {
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let dist = (dx * dx + dy * dy).sqrt();
    if !dist.is_finite() || dist < MIN_PX {
        return Glide {
            steps: vec![[to_x, to_y]],
        };
    }
    let ms = (dist * MS_PER_PX).clamp(MIN_MS, MAX_MS);
    let n = (ms / STEP_MS as f64).round().max(1.0) as usize;
    let mut steps = Vec::with_capacity(n);
    for i in 1..=n {
        let u = i as f64 / n as f64;
        let e = u * u * (3.0 - 2.0 * u);
        steps.push([from_x + dx * e, from_y + dy * e]);
    }
    if let Some(last) = steps.last_mut() {
        *last = [to_x, to_y];
    }
    Glide { steps }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_move_starts_near_the_pointer_and_lands_exactly() {
        let glide = samples(0.0, 0.0, 400.0, 0.0);
        assert!(glide.steps.len() > 1);
        let first = glide.steps[0][0];
        assert!(
            first > 0.0 && first < 80.0,
            "first step {first} jumped ahead"
        );
        assert_eq!(*glide.steps.last().unwrap(), [400.0, 0.0]);
        let mut previous = 0.0;
        for step in &glide.steps {
            assert!(step[0] + 1e-9 >= previous);
            assert_eq!(step[1], 0.0);
            previous = step[0];
        }
    }

    #[test]
    fn medium_move_has_no_warp_and_ends_on_the_target() {
        let glide = samples(10.0, 10.0, 50.0, 10.0);
        assert!(glide.steps.len() > 1);
        assert!(glide.steps[0][0] < 25.0);
        assert_eq!(*glide.steps.last().unwrap(), [50.0, 10.0]);
    }

    #[test]
    fn tiny_move_is_only_the_target() {
        let glide = samples(1.0, 1.0, 4.0, 1.0);
        assert_eq!(glide.steps, vec![[4.0, 1.0]]);
    }

    #[test]
    fn sample_count_follows_the_duration_clamp() {
        let short = samples(0.0, 0.0, 40.0, 0.0);
        let capped = samples(0.0, 0.0, 4_000.0, 0.0);
        assert_eq!(
            short.steps.len(),
            (MIN_MS / STEP_MS as f64).round() as usize
        );
        assert_eq!(
            capped.steps.len(),
            (MAX_MS / STEP_MS as f64).round() as usize
        );
        assert_eq!(*capped.steps.last().unwrap(), [4_000.0, 0.0]);
        assert!(capped.steps[0][0] < 400.0);
    }
}
