use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

use crate::ai;
use crate::commands;
use crate::db::{Db, Zone};

pub const STATUS_EVENT: &str = "zone-behavior-updated";
pub const READS_PER_BATCH: usize = 10;
const ARTICLE_ACTIVITY_QUIET_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct BehaviorLearningControl {
    active_zones: Arc<AsyncMutex<HashSet<i64>>>,
    slot: Arc<Semaphore>,
    article_activity: Arc<AtomicU64>,
}

impl BehaviorLearningControl {
    pub fn new() -> Self {
        Self {
            active_zones: Arc::new(AsyncMutex::new(HashSet::new())),
            slot: Arc::new(Semaphore::new(1)),
            article_activity: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BehaviorUpdate {
    pub zone_id: i64,
    pub phase: String,
    pub zone: Option<Zone>,
}

pub fn start(app_handle: AppHandle) {
    queue_ready(app_handle);
}

pub fn queue_ready(app_handle: AppHandle) {
    let zone_ids = {
        let db = app_handle.state::<Db>();
        match db.ready_behavior_zone_ids(READS_PER_BATCH) {
            Ok(zone_ids) => zone_ids,
            Err(err) => {
                eprintln!("Could not find Xones ready for behavior learning: {err}");
                return;
            }
        }
    };
    for zone_id in zone_ids {
        queue_zone(app_handle.clone(), zone_id);
    }
}

pub fn note_article_activity(app_handle: &AppHandle) {
    app_handle
        .state::<BehaviorLearningControl>()
        .article_activity
        .fetch_add(1, Ordering::Relaxed);
}

pub fn queue_zone(app_handle: AppHandle, zone_id: i64) {
    tauri::async_runtime::spawn(async move {
        let control = app_handle
            .state::<BehaviorLearningControl>()
            .inner()
            .clone();
        {
            let mut active = control.active_zones.lock().await;
            if !active.insert(zone_id) {
                return;
            }
        }

        wait_for_article_quiet(&control).await;

        let permit = control
            .slot
            .acquire()
            .await
            .expect("behavior learning semaphore remains open");
        let progressed = process_zone(&app_handle, zone_id).await;
        drop(permit);
        control.active_zones.lock().await.remove(&zone_id);

        let still_ready = {
            let db = app_handle.state::<Db>();
            db.ready_behavior_zone_ids(READS_PER_BATCH)
                .map(|zone_ids| zone_ids.contains(&zone_id))
                .unwrap_or(false)
        };
        // Requeue only after real progress. If the pass completed no batch,
        // rescheduling it could not do better and would spin forever.
        if progressed && still_ready {
            queue_zone(app_handle, zone_id);
        }
    });
}

async fn wait_for_article_quiet(control: &BehaviorLearningControl) {
    let mut observed_activity = control.article_activity.load(Ordering::Relaxed);
    loop {
        tokio::time::sleep(ARTICLE_ACTIVITY_QUIET_PERIOD).await;
        let current_activity = control.article_activity.load(Ordering::Relaxed);
        if current_activity == observed_activity {
            return;
        }
        observed_activity = current_activity;
    }
}

/// Returns whether at least one batch was learned and saved.
async fn process_zone(app_handle: &AppHandle, zone_id: i64) -> bool {
    let mut completed_batches = 0usize;
    loop {
        let batch = {
            let db = app_handle.state::<Db>();
            match db.next_behavior_batch(zone_id, READS_PER_BATCH) {
                Ok(Some(batch)) => batch,
                Ok(None) => return completed_batches > 0,
                Err(err) => {
                    eprintln!("Could not load behavior-learning batch: {err}");
                    emit_update(app_handle, zone_id, "failed");
                    return false;
                }
            }
        };
        let settings = {
            let db = app_handle.state::<Db>();
            match db.get_settings() {
                Ok(rows) => ai::settings_from_rows(rows),
                Err(err) => {
                    record_failure(app_handle, zone_id, batch.revision, &err);
                    return false;
                }
            }
        };

        match ai::learn_behavior_profile(
            &settings,
            &batch.filter_description,
            &batch.current_profile,
            &batch.articles,
        )
        .await
        {
            Ok(result) => {
                let completed = {
                    let db = app_handle.state::<Db>();
                    db.complete_behavior_batch(&batch, &result.profile)
                };
                match completed {
                    Ok(true) => {
                        completed_batches += 1;
                        emit_update(app_handle, zone_id, "completed");
                    }
                    Ok(false) => return completed_batches > 0,
                    Err(err) => {
                        record_failure(app_handle, zone_id, batch.revision, &err);
                        return false;
                    }
                }
            }
            Err(err) => {
                record_failure(app_handle, zone_id, batch.revision, &err);
                return false;
            }
        }
    }
}

fn record_failure(app_handle: &AppHandle, zone_id: i64, revision: i64, error: &str) {
    let compact = commands::compact_error(error);
    let db = app_handle.state::<Db>();
    if let Err(err) = db.fail_behavior_batch(zone_id, revision, &compact) {
        eprintln!("Could not save behavior-learning failure: {err}");
    }
    emit_update(app_handle, zone_id, "failed");
}

pub fn emit_update(app_handle: &AppHandle, zone_id: i64, phase: &str) {
    let zone = app_handle
        .state::<Db>()
        .get_zone(zone_id)
        .unwrap_or_else(|err| {
            eprintln!("Could not load Xone after behavior update: {err}");
            None
        });
    let _ = app_handle.emit(
        STATUS_EVENT,
        BehaviorUpdate {
            zone_id,
            phase: phase.to_string(),
            zone,
        },
    );
}
