//! Identify when the Tokio workers are too busy to handle more work.
//!
//! ### Monitor the Tokio workers' busy ratio
//! [TooBusy] allows you to track the busy ratio of the Tokio workers and react accordingly
//! when they are too busy.
//!
//! In the example below, a [`TooBusy`] is [built][TooBusy::builder] and used to
//! [evaluate][TooBusy::eval] the busy ratio of the Tokio workers and progressively reject requests
//! when the busy ratio goes above the [low watermark][TooBusyBuilder::low_watermark] threshold:
//! ```
//!async fn my_middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
//!
//!   if state.too_busy.eval() {
//!       return (StatusCode::SERVICE_UNAVAILABLE, "Server is too busy").into_response();
//!   }
//!   next.run(request).await
//!}
//!
//!#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
//!async fn main() {
//!   let too_busy = TooBusy::builder()
//!       .interval(Duration::from_secs(1))
//!       .ewma_alpha(0.1)
//!       .high_watermark(95)
//!       .low_watermark(90)
//!       .build();
//!
//!   let state = AppState { too_busy };
//!
//!   let app = Router::new()
//!      .route("/", get(root))
//!      .route_layer(middleware::from_fn_with_state(state.clone(), my_middleware))
//!      .with_state(state);
//!
//!  let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
//!  axum::serve(listener, app).await.unwrap();
//!}
//! ```
//! Check the full example in the [examples] directory.
//!
//! Tokio too busy leverages the [worker_total_busy_duration] interface provided by the Tokio [Runtime] for knowledge.
//! and its only available when Tokio is compiled using the following flags ```RUSTFLAGS="--cfg tokio_unstable"```.
//!
//! Busy ratio is calculated by averaging the busy duration of all workers since latest iteration and using an 
//! exponential moving average (EWMA).
//!
//! [examples]: https://github.com/pfreixes/tokio-too-busy/tree/main/examples
//! [worker_total_busy_duration]: https://docs.rs/tokio/1.0.1/tokio/runtime/struct.Runtime.html#method.worker_total_busy_duration
//! [Runtime]: https://docs.rs/tokio/1.0.1/tokio/runtime/struct.Runtime.html
mod inner;
use crate::inner::{LoadFeeder, TooBusyShared};
use std::sync::Arc;
use tokio::time::Duration;

/// Track how busy are the tokio workers.
///
/// This type is internally reference-counted and can be freely cloned.
pub struct TooBusy {
    inner: Arc<TooBusyShared>,
}

impl TooBusy {
    /// Create a new builder to configure the [`TooBusy`] instance.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio_too_busy::TooBusy;
    /// use std::time::Duration;
    ///
    /// let too_busy = TooBusy::builder()
    ///     .interval(Duration::from_secs(1))
    ///     .ewma_alpha(0.1)
    ///     .high_watermark(95)
    ///     .low_watermark(90)
    ///     .build();
    /// ```
    pub fn builder() -> TooBusyBuilder {
        TooBusyBuilder::default()
    }

    /// Get the current busy ratio of the Tokio workers.
    pub fn ratio_busy_ewma(&self) -> u32 {
        self.inner.ratio_busy_ewma()
    }

    /// Evaluate if the Tokio workers are too busy.
    ///
    /// Returns `true` if the busy ratio is above the low watermark threshold using
    /// a progressive rejection strategy until reach the high watermark threshold, above
    /// the high watermark all evaluations will return `true`. Otherwise, it returns `false`.
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

/// This builder allows you to configure the [`TooBusy`] instance.
pub struct TooBusyBuilder {
    interval: Duration,
    low_watermark: u32,
    high_watermark: u32,
    ewma_alpha: f32,
}

impl TooBusyBuilder {
    /// Set the interval to update the busy ratio.
    ///
    /// By default, it is 1 seconds.
    pub fn interval(mut self, interval: Duration) -> TooBusyBuilder {
        self.interval = interval;
        self
    }
    /// Set the low watermark threshold to start considering the workers too busy.
    ///
    /// By default, it is 85%.
    pub fn low_watermark(mut self, low_watermark: u32) -> TooBusyBuilder {
        assert!(
            low_watermark < self.high_watermark,
            "low_watermark must be lower than high_watermark!"
        );
        self.low_watermark = low_watermark;
        self
    }
    /// Set the high watermark threshold to start considering the workers too busy all the time.
    ///
    /// By default, it is 95%.
    pub fn high_watermark(mut self, high_watermark: u32) -> TooBusyBuilder {
        assert!(
            high_watermark > self.low_watermark,
            "high_watermark must be greater than low_watermark!"
        );
        self.high_watermark = high_watermark;
        self
    }
    /// Set the alpha value for the EWMA calculation. 
    ///
    /// By default, it is 0.1 (10%) for having quick reactions to changes. If you want to smooth the changes
    /// you can set a higher value.
    pub fn ewma_alpha(mut self, ewma_alpha: f32) -> TooBusyBuilder {
        assert!(
            ewma_alpha > 0.0 && ewma_alpha < 1.0,
            "ewma_alpha must be between the range (0, 1)!"
        );
        self.ewma_alpha = ewma_alpha;
        self
    }
    /// Build the [`TooBusy`] instance.
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
            interval: Duration::from_secs(1),
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
