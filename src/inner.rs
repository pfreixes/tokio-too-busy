use rand::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Weak;
use std::time::Instant;
use tokio::time::{interval_at, Duration};

pub(crate) struct TooBusyShared {
    low_watermark: u32,
    high_watermark: u32,
    ratio_busy_ewma: AtomicU32,
}

impl TooBusyShared {
    pub(crate) fn new(low_watermark: u32, high_watermark: u32) -> Self {
        TooBusyShared {
            low_watermark,
            high_watermark,
            ratio_busy_ewma: AtomicU32::new(0),
        }
    }

    pub(crate) fn ratio_busy_ewma(&self) -> u32 {
        self.ratio_busy_ewma.load(Ordering::Relaxed)
    }

    pub(crate) fn eval(&self) -> bool {
        let ratio_busy_ewma = self.ratio_busy_ewma.load(Ordering::Relaxed);
        self.calculate_proabilistic_too_busy(&mut rand::thread_rng(), ratio_busy_ewma)
    }

    fn calculate_proabilistic_too_busy<A: Rng>(&self, rng: &mut A, ratio_busy_ewma: u32) -> bool {
        if ratio_busy_ewma < self.low_watermark {
            return false;
        } else if ratio_busy_ewma >= self.high_watermark {
            return true;
        }

        // we are in the middle of the low and high
        // watermark, we will return too busy progressivelly
        // from [0, 99] % depending on value of the current load.
        let max_range = self.high_watermark - self.low_watermark;

        // tell us the percentage of calls that would need to return too busy.
        let percentage = ((ratio_busy_ewma - self.low_watermark) * 100 / max_range) as u32;

        if rng.gen::<u32>() % 100 < percentage {
            return true;
        } else {
            return false;
        }
    }
}

pub(crate) struct LoadFeeder {
    pub(crate) inner: Weak<TooBusyShared>,
    interval: Duration,
    ewma_alpha: f32,
}

impl LoadFeeder {
    pub(crate) fn new(inner: Weak<TooBusyShared>, interval: Duration, ewma_alpha: f32) -> Self {
        LoadFeeder {
            inner,
            interval,
            ewma_alpha,
        }
    }

    pub(crate) async fn run(&self) {
        let metrics = tokio::runtime::Handle::current().metrics();
        let num_workers = metrics.num_workers() as u32;
        let start = Instant::now() + self.interval;
        let mut interval = interval_at(start.into(), self.interval);
        let mut ratio_busy_ewma: f32 = 0.0;
        let mut latest_total_busy_accumulated = ((0..num_workers as usize)
            .map(|worker| metrics.worker_total_busy_duration(worker))
            .sum::<Duration>()
            / num_workers).as_millis();

        loop {
            let _ = interval.tick().await;
            let too_busy_shared = match self.inner.upgrade() {
                Some(inner) => inner,
                None => break,
            };

            let total_busy_accumulated = ((0..num_workers as usize)
                .map(|worker| metrics.worker_total_busy_duration(worker))
                .sum::<Duration>()
                / num_workers).as_millis();

            let total_busy_since_last_iteration = total_busy_accumulated - latest_total_busy_accumulated;
            latest_total_busy_accumulated = total_busy_accumulated;
            ratio_busy_ewma = self.calculate_ratio_busy_ewma(total_busy_since_last_iteration, ratio_busy_ewma);

            too_busy_shared
                .ratio_busy_ewma
                .store(ratio_busy_ewma as u32, Ordering::Relaxed);
        }
    }

    fn calculate_ratio_busy_ewma(&self, busy: u128, ratio_busy_ewma: f32) -> f32 {
        let ratio_busy = f32::min(
            100.0,
            (busy as f32 / self.interval.as_millis() as f32) * 100.0,
        );
        (self.ewma_alpha * ratio_busy_ewma) + ((1.0 - self.ewma_alpha) * ratio_busy)
    }

    
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::mock::StepRng;
    use std::sync::Arc;

    macro_rules! too_busy_shared_tests {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (too_busy_shared, rng, ratio_busy_ewma, expected) = $value;
                assert_eq!(expected, too_busy_shared.calculate_proabilistic_too_busy(rng, ratio_busy_ewma));
            }
        )*
        }
    }

    too_busy_shared_tests! {
        too_busy_shared_tests_below_low_watermark: (TooBusyShared::new(80, 90), &mut StepRng::new(99, 1), 79, false),
        too_busy_shared_tests_within_watermarks_randomly_false: (TooBusyShared::new(80, 90), &mut StepRng::new(50, 1), 85, false),
        too_busy_shared_tests_within_watermarks_randomly_true: (TooBusyShared::new(80, 90), &mut StepRng::new(49, 1), 85, true),
        too_busy_shared_tests_above_high_watermark: (TooBusyShared::new(80, 90), &mut StepRng::new(0, 1), 90, true),
    }

    macro_rules! load_feeder_tests {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (ewma_alpha, interval, busy, ratio_busy_ewma, expected) = $value;
                let inner = Arc::new(TooBusyShared::new(80, 90));
                let weak_reference = Arc::downgrade(&inner);
                let load_feeder = LoadFeeder::new(weak_reference, interval, ewma_alpha);
                assert_eq!(expected, load_feeder.calculate_ratio_busy_ewma(busy.as_millis(), ratio_busy_ewma));
            }
        )*
        }
    }

    load_feeder_tests! {
        load_feeder_tests_alpha_0_9: (0.9, Duration::from_secs(10), Duration::from_secs(5), 10.0, 14.000001),
        load_feeder_tests_alpha_0_1: (0.1, Duration::from_secs(10), Duration::from_secs(5), 10.0, 46.0),
        load_feeder_tests_alpha_max_100: (0.1, Duration::from_secs(10), Duration::from_secs(11), 10.0, 91.0),
    }
}
