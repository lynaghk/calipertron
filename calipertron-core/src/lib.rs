#![no_std]

use core::f32::consts::PI;

/// Exponential moving average over (sine, cosine) correlation sums. Averaging in phasor
/// space (before atan2) is immune to the wraparound artifacts that averaging raw phases
/// would produce near +/-pi.
pub struct PhasorEma {
    alpha: f32,
    state: Option<(f32, f32)>,
}

impl PhasorEma {
    pub fn new(alpha: f32) -> Self {
        PhasorEma { alpha, state: None }
    }

    /// Blend in a new phasor and return the smoothed one. The first phasor after
    /// construction or reset() seeds the average unchanged.
    pub fn update(&mut self, (sine, cosine): (f32, f32)) -> (f32, f32) {
        let smoothed = match self.state {
            None => (sine, cosine),
            Some((s, c)) => (
                self.alpha * sine + (1.0 - self.alpha) * s,
                self.alpha * cosine + (1.0 - self.alpha) * c,
            ),
        };
        self.state = Some(smoothed);
        smoothed
    }

    /// Forget the average. Call whenever the phase reference breaks (e.g. the signal was
    /// stopped), or stale state would blend two unrelated reference frames.
    pub fn reset(&mut self) {
        self.state = None;
    }
}

pub struct PhaseAccumulator {
    pub unwrapped_phase: f32,
    last_phase: f32,
    hysteresis_threshold: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phasor_ema_seeds_with_first_phasor() {
        let mut ema = PhasorEma::new(0.1);
        assert_eq!(ema.update((3.0, 4.0)), (3.0, 4.0));
    }

    #[test]
    fn phasor_ema_blends_subsequent_phasors() {
        let mut ema = PhasorEma::new(0.1);
        ema.update((1.0, 0.0));
        let (sine, cosine) = ema.update((0.0, 1.0));
        assert!((sine - 0.9).abs() < 1e-6);
        assert!((cosine - 0.1).abs() < 1e-6);
    }

    #[test]
    fn phasor_ema_reset_reseeds_with_next_phasor() {
        let mut ema = PhasorEma::new(0.1);
        ema.update((1.0, 0.0));
        ema.reset();
        assert_eq!(ema.update((0.0, 1.0)), (0.0, 1.0));
    }
}

impl PhaseAccumulator {
    pub fn new(initial_phase: f32, hysteresis_threshold: f32) -> Self {
        PhaseAccumulator {
            unwrapped_phase: 0.0,
            last_phase: initial_phase,
            hysteresis_threshold,
        }
    }

    pub fn update(&mut self, new_phase: f32) {
        let mut delta = new_phase - self.last_phase;

        // Handle wraparound
        if delta > PI {
            delta -= 2.0 * PI;
        } else if delta < -PI {
            delta += 2.0 * PI;
        }

        // Apply hysteresis
        if delta.abs() > self.hysteresis_threshold {
            self.unwrapped_phase += delta;
            self.last_phase = new_phase;
        }
    }
}
