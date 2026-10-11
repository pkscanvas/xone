//! Background drain for the classification backlog.
//!
//! A refresh cycle classifies only the newest articles per Xone (see
//! `commands::COLD_START_BURST_LIMIT`) so a fresh list appears quickly even
//! after the app has been closed for days. Whatever the cycle left behind is
//! worked through here at a deliberately slow, paced rate, so catching up
//! never competes with what the user is reading right now.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use tokio::time::sleep;

use crate::ai;
use crate::commands::{self, ClassifyPass, ClassifyZoneFailure};
use crate::db::Db;
use crate::scheduler::{AutoRefreshStatus, STATUS_EVENT};

/// Articles classified per acquisition from the bucket. Small enough that the
/// drain yields often, large enough to keep the fan-out worthwhile.
const CHUNK: usize = 10;
/// Sustained drain rate. At one article every two seconds a 500-article
/// backlog clears in under twenty minutes without ever saturating the
/// provider or the local machine.
const REFILL_PER_SECOND: f64 = 0.5;
/// Allows a chunk to start immediately when the drain has been idle.
const BUCKET_CAPACITY: f64 = CHUNK as f64;
/// Backstop against a zone that somehow never reports its backlog shrinking.
const MAX_CHUNKS_PER_ZONE: usize = 200;

/// Guards against two drains running at once. A second request while one is
/// already working is a no-op: the running drain will pick up the newly
/// unclassified articles on its own.
#[derive(Clone, Default)]
pub struct DrainControl {
    running: Arc<AtomicBool>,
}

impl DrainControl {
    pub fn new() -> Self {
        Self::default()
    }

    fn try_claim(&self) -> bool {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn release(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Start draining if a drain is not already in progress.
pub fn start(app_handle: AppHandle) {
    let control = app_handle.state::<DrainControl>().inner().clone();
    if !control.try_claim() {
        return;
    }

    tauri::async_runtime::spawn(async move {
        run(&app_handle).await;
        control.release();
    });
}

async fn run(app_handle: &AppHandle) {
    let Some((settings, processing_limit, zone_ids)) = load_work(app_handle) else {
        return;
    };
    if !ai::can_classify(&settings) || zone_ids.is_empty() {
        return;
    }

    let mut bucket = TokenBucket::new();
    let mut status = AutoRefreshStatus::drain(processing_limit);
    status.zones_total = zone_ids.len();

    for zone_id in zone_ids {
        for _ in 0..MAX_CHUNKS_PER_ZONE {
            bucket.acquire(CHUNK).await;

            let outcome = {
                let db = app_handle.state::<Db>();
                commands::classify_zone_with_settings(
                    &db,
                    &settings,
                    zone_id,
                    ClassifyPass::background(processing_limit, CHUNK),
                )
                .await
            };

            let Ok(result) = outcome else {
                // The Xone was deleted mid-drain, or its row could not be
                // read. Either way there is nothing left to drain here.
                break;
            };

            status.articles_classified += result.articles_classified;
            status.articles_relevant += result.articles_relevant;
            for failure in result.failures {
                push_unique(&mut status.failures, failure);
            }
            status.articles_queued = result.articles_pending;
            emit(app_handle, &status);

            // `articles_pending` is forced to zero when nothing in the chunk
            // succeeded, so an unreachable provider ends the drain rather than
            // retrying the same failure hundreds of times.
            if result.articles_pending == 0 {
                break;
            }
        }
        status.zones_completed += 1;
    }

    if status.articles_classified == 0 && status.failures.is_empty() {
        // Nothing was left to do after all; stay quiet rather than making the
        // UI reload for no reason.
        return;
    }

    status.articles_queued = 0;
    status.phase = "completed".to_string();
    status.completed_at = Some(chrono::Utc::now().to_rfc3339());
    emit(app_handle, &status);
}

/// Snapshot everything the drain needs before it starts awaiting, so the
/// database guard is never held across a request.
fn load_work(app_handle: &AppHandle) -> Option<(ai::AiSettings, usize, Vec<i64>)> {
    let db = app_handle.state::<Db>();
    let settings = db.get_settings().map(ai::settings_from_rows).ok()?;
    let processing_limit = commands::classification_processing_limit(&db).ok()?;
    let zone_ids = db
        .list_zones()
        .ok()?
        .into_iter()
        .map(|zone| zone.id)
        .collect();
    Some((settings, processing_limit, zone_ids))
}

fn push_unique(failures: &mut Vec<ClassifyZoneFailure>, failure: ClassifyZoneFailure) {
    if failures.len() >= 3 || failures.iter().any(|seen| seen.message == failure.message) {
        return;
    }
    failures.push(failure);
}

fn emit(app_handle: &AppHandle, status: &AutoRefreshStatus) {
    if let Err(err) = app_handle.emit(STATUS_EVENT, status.clone()) {
        eprintln!("Could not emit drain status: {err}");
    }
}

/// Paces the drain. Tokens accrue while the drain is idle, so a small backlog
/// clears in one burst while a large one settles into the sustained rate.
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new() -> Self {
        Self {
            tokens: BUCKET_CAPACITY,
            last_refill: Instant::now(),
        }
    }

    async fn acquire(&mut self, count: usize) {
        let needed = count as f64;
        loop {
            self.refill();
            if self.tokens >= needed {
                self.tokens -= needed;
                return;
            }
            let wait = (needed - self.tokens) / REFILL_PER_SECOND;
            sleep(Duration::from_secs_f64(wait.clamp(0.05, 30.0))).await;
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * REFILL_PER_SECOND).min(BUCKET_CAPACITY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bucket_refills_at_the_sustained_rate_and_stops_at_capacity() {
        let mut bucket = TokenBucket::new();
        bucket.tokens = 0.0;
        bucket.last_refill = Instant::now() - Duration::from_secs(4);

        bucket.refill();
        assert!(
            (bucket.tokens - 2.0).abs() < 0.1,
            "tokens: {}",
            bucket.tokens
        );

        bucket.last_refill = Instant::now() - Duration::from_secs(600);
        bucket.refill();
        assert_eq!(bucket.tokens, BUCKET_CAPACITY);
    }

    #[test]
    fn a_second_drain_request_is_a_no_op_while_one_is_running() {
        let control = DrainControl::new();
        assert!(control.try_claim());
        assert!(!control.try_claim());
        control.release();
        assert!(control.try_claim());
    }
}
