mod inner;
use crate::inner::{LoadFeeder, TooBusyShared};
use std::sync::Arc;
use tokio::time::Duration;

pub struct TooBusy {
    inner: Arc<TooBusyShared>,
}

impl TooBusy {
    pub fn builder() -> TooBusyBuilder {
        TooBusyBuilder::default()
    }

    pub fn ratio_busy_ewma(&self) -> u32 {
        self.inner.ratio_busy_ewma()
    }

    pub fn eval(&self) -> bool {
        self.inner.eval()
    }
}

impl Clone for TooBusy {
    fn clone(&self) -> Self {
        TooBusy {
            inner: self.inner.clone(),
        }
    }
}

pub struct TooBusyBuilder {
    interval: Duration,
    low_watermark: u32,
    high_watermark: u32,
    ewma_alpha: f32,
}

impl TooBusyBuilder {
    pub fn interval(mut self, interval: Duration) -> TooBusyBuilder {
        self.interval = interval;
        self
    }

    pub fn low_watermark(mut self, low_watermark: u32) -> TooBusyBuilder {
        assert!(
            low_watermark < self.high_watermark,
            "low_watermark must be lower than high_watermark!"
        );
        self.low_watermark = low_watermark;
        self
    }

    pub fn high_watermark(mut self, high_watermark: u32) -> TooBusyBuilder {
        assert!(
            high_watermark > self.low_watermark,
            "high_watermark must be greater than low_watermark!"
        );
        self.high_watermark = high_watermark;
        self
    }

    pub fn ewma_alpha(mut self, ewma_alpha: f32) -> TooBusyBuilder {
        assert!(
            ewma_alpha > 0.0 && ewma_alpha < 1.0,
            "ewma_alpha must be between the range (0, 1)!"
        );
        self.ewma_alpha = ewma_alpha;
        self
    }

    pub fn build(self) -> TooBusy {
        let inner = Arc::new(TooBusyShared::new(self.low_watermark, self.high_watermark));
        let weak_reference = Arc::downgrade(&inner);

        tokio::spawn(async move {
            LoadFeeder::new(weak_reference, self.interval, self.ewma_alpha)
                .run()
                .await;
        });

        TooBusy { inner }
    }
}

impl Default for TooBusyBuilder {
    fn default() -> Self {
        TooBusyBuilder {
            interval: Duration::from_secs(5),
            low_watermark: 85,
            high_watermark: 95,
            ewma_alpha: 0.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn too_busy_builder_invalid_low_watermark() {
        TooBusyBuilder::default()
            .high_watermark(90)
            .low_watermark(90);
    }

    #[test]
    #[should_panic]
    fn too_busy_builder_invalid_high_watermark() {
        TooBusyBuilder::default()
            .low_watermark(90)
            .high_watermark(90);
    }

    #[test]
    #[should_panic]
    fn too_busy_builder_invalid_ewma_alpha_too_low() {
        TooBusyBuilder::default().ewma_alpha(0.0);
    }

    #[test]
    #[should_panic]
    fn too_busy_builder_invalid_ewma_alpha_too_high() {
        TooBusyBuilder::default().ewma_alpha(1.0);
    }
}
