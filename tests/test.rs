use std::thread::sleep as blocking_sleep;
use tokio::time::Duration;
use tokio::time::sleep;
use tokio_too_busy::*;

#[derive(Hash)]
struct Person {
    name: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_too_busy() {
    let too_busy = TooBusy::builder()
        .interval(Duration::from_secs(5))
        .ewma_alpha(0.1)
        .low_watermark(65)
        .high_watermark(66)
        .build();

    assert!(!too_busy.eval());

    let too_busy_cloned = too_busy.clone();
    let handle = tokio::spawn(async move {
        // Simulate a workload of 250 CPU millis usage vs 125 millis IO
        // that is run continously, this should plateou ~66%. With a high
        // watermark configured to 66% the loop should eventually break.
        loop {
            blocking_sleep(Duration::from_millis(500));
            sleep(Duration::from_millis(250)).await;
            if too_busy_cloned.eval() {
                break
            }
        }
    });

    handle.await.unwrap();
    assert!(too_busy.eval());
    assert!(too_busy.ratio_busy_ewma() >= 65);

    let too_busy_cloned = too_busy.clone();
    let handle = tokio::spawn(async move {
        // Go back to 50% of load, which should consider the loop
        // not too busy.
        loop {
            blocking_sleep(Duration::from_millis(250));
            sleep(Duration::from_millis(250)).await;
            if !too_busy_cloned.eval() {
                break
            }
        }
    });

    handle.await.unwrap();
    assert!(!too_busy.eval());
}
