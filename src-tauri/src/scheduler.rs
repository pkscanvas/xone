use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::{stream, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Mutex as AsyncMutex, OnceCell, Semaphore};

use crate::ai;
use crate::commands::{self, ClassifyZoneFailure, FeedRefreshResult};
use crate::db::{Db, Feed, Zone};
use crate::drain;

pub const STATUS_EVENT: &str = "auto-refresh-status";
const LAST_AUTO_REFRESH_AT_KEY: &str = "last_auto_refresh_at";
const MAX_CONCURRENT_ZONE_WORKFLOWS: usize = 2;

type SharedFeedRefresh = Arc<OnceCell<Result<FeedRefreshResult, String>>>;

struct ZoneWorkflow {
    zone: Zone,
    feeds: Vec<Feed>,
}

#[derive(Clone)]
pub struct SchedulerControl {
    reschedule_tx: watch::Sender<u64>,
}

impl SchedulerControl {
    pub fn new() -> Self {
        let (reschedule_tx, _) = watch::channel(0);
        Self { reschedule_tx }
    }

    pub fn reschedule(&self) {
        self.reschedule_tx.send_modify(|version| {
            *version = version.wrapping_add(1);
        });
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.reschedule_tx.subscribe()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AutoRefreshStatus {
    pub phase: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub sources_total: usize,
    pub sources_completed: usize,
    pub sources_refreshed: usize,
    pub articles_fetched: usize,
    pub articles_inserted: usize,
    pub zones_total: usize,
    pub zones_completed: usize,
    pub articles_classified: usize,
    pub articles_relevant: usize,
    pub articles_queued: usize,
    pub processing_limit: usize,
    /// Set when this status describes the paced background drain rather than a
    /// refresh cycle, so the run-details popover can label it correctly.
    pub is_drain: bool,
    pub failures: Vec<ClassifyZoneFailure>,
}

impl AutoRefreshStatus {
    fn new(processing_limit: usize) -> Self {
        Self {
            phase: "processing_zones".to_string(),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            sources_total: 0,
            sources_completed: 0,
            sources_refreshed: 0,
            articles_fetched: 0,
            articles_inserted: 0,
            zones_total: 0,
            zones_completed: 0,
            articles_classified: 0,
            articles_relevant: 0,
            articles_queued: 0,
            processing_limit,
            is_drain: false,
            failures: Vec::new(),
        }
    }

    pub fn drain(processing_limit: usize) -> Self {
        Self {
            is_drain: true,
            ..Self::new(processing_limit)
        }
    }
}

pub fn start(app_handle: AppHandle) {
    let mut reschedule_rx = app_handle.state::<SchedulerControl>().subscribe();

    tauri::async_runtime::spawn(async move {
        loop {
            let interval_minutes = {
                let db = app_handle.state::<Db>();
                commands::refresh_interval_minutes(&db).unwrap_or(30)
            };
            let delay = {
                let db = app_handle.state::<Db>();
                next_delay(&db, interval_minutes)
            };

            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    run_cycle(&app_handle).await;
                }
                changed = reschedule_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

async fn run_cycle(app_handle: &AppHandle) {
    let db = app_handle.state::<Db>();
    let processing_limit = commands::classification_processing_limit(&db).unwrap_or(0);
    let mut status = AutoRefreshStatus::new(processing_limit);
    if let Err(err) = db.update_setting(LAST_AUTO_REFRESH_AT_KEY, &status.started_at) {
        status.failures.push(ClassifyZoneFailure {
            source: "Scheduler".to_string(),
            message: commands::compact_error(&err),
        });
    }

    let workflows = match load_zone_workflows(&db) {
        Ok(workflows) => workflows,
        Err(err) => {
            fail_cycle(app_handle, &mut status, "Xones", err);
            return;
        }
    };
    status.sources_total = unique_source_count(&workflows);
    status.zones_total = workflows.len();

    let ai_settings = match db.get_settings().map(ai::settings_from_rows) {
        Ok(settings) => settings,
        Err(err) => {
            fail_cycle(app_handle, &mut status, "AI settings", err);
            return;
        }
    };
    drop(db);

    emit_status(app_handle, &status);
    let status = Arc::new(AsyncMutex::new(status));
    let feed_slots = Arc::new(Semaphore::new(commands::MAX_CONCURRENT_FEED_REFRESHES));
    let feed_refreshes = shared_feed_refreshes(&workflows);
    let ai_settings = Arc::new(ai_settings);

    stream::iter(workflows.into_iter().map(|workflow| {
        process_zone_workflow(
            app_handle.clone(),
            workflow,
            Arc::clone(&ai_settings),
            processing_limit,
            Arc::clone(&feed_slots),
            feed_refreshes.clone(),
            Arc::clone(&status),
        )
    }))
    .buffer_unordered(MAX_CONCURRENT_ZONE_WORKFLOWS)
    .collect::<Vec<_>>()
    .await;

    let completed_at = Utc::now().to_rfc3339();
    {
        let db = app_handle.state::<Db>();
        if let Err(err) = db.update_setting(LAST_AUTO_REFRESH_AT_KEY, &completed_at) {
            let mut status = status.lock().await;
            status.failures.push(ClassifyZoneFailure {
                source: "Scheduler".to_string(),
                message: commands::compact_error(&err),
            });
        }
        if let Err(err) = commands::cleanup_old_articles_if_due(&db) {
            let mut status = status.lock().await;
            status.failures.push(ClassifyZoneFailure {
                source: "Article cleanup".to_string(),
                message: commands::compact_error(&err),
            });
        }
    }
    let completed = {
        let mut status = status.lock().await;
        status.phase = "completed".to_string();
        status.completed_at = Some(completed_at);
        status.clone()
    };
    emit_status(app_handle, &completed);

    // Each Xone classified only its newest articles. Work through whatever is
    // left over at a paced rate, well behind the user's own activity.
    if completed.articles_queued > 0 {
        drain::start(app_handle.clone());
    }
}

fn load_zone_workflows(db: &Db) -> Result<Vec<ZoneWorkflow>, String> {
    db.list_zones()?
        .into_iter()
        .map(|zone| {
            let feeds = db.get_zone_feeds(zone.id)?;
            Ok(ZoneWorkflow { zone, feeds })
        })
        .collect()
}

fn unique_source_count(workflows: &[ZoneWorkflow]) -> usize {
    workflows
        .iter()
        .flat_map(|workflow| workflow.feeds.iter().map(|feed| feed.id))
        .collect::<HashSet<_>>()
        .len()
}

fn shared_feed_refreshes(workflows: &[ZoneWorkflow]) -> Arc<HashMap<i64, SharedFeedRefresh>> {
    Arc::new(
        workflows
            .iter()
            .flat_map(|workflow| workflow.feeds.iter())
            .map(|feed| (feed.id, Arc::new(OnceCell::new())))
            .collect(),
    )
}

async fn process_zone_workflow(
    app_handle: AppHandle,
    workflow: ZoneWorkflow,
    ai_settings: Arc<ai::AiSettings>,
    processing_limit: usize,
    feed_slots: Arc<Semaphore>,
    feed_refreshes: Arc<HashMap<i64, SharedFeedRefresh>>,
    status: Arc<AsyncMutex<AutoRefreshStatus>>,
) {
    let mut refreshes = stream::iter(workflow.feeds.into_iter().map(|feed| {
        let app_handle = app_handle.clone();
        let feed_slots = Arc::clone(&feed_slots);
        let refresh = Arc::clone(
            feed_refreshes
                .get(&feed.id)
                .expect("zone source has a shared refresh cell"),
        );
        let status = Arc::clone(&status);

        async move {
            refresh
                .get_or_init(|| async {
                    let _permit = feed_slots
                        .acquire()
                        .await
                        .expect("feed refresh semaphore remains open");
                    let result = {
                        let db = app_handle.state::<Db>();
                        commands::refresh_feed_without_classification(&db, feed.id).await
                    };
                    record_source_result(&app_handle, &status, &feed, &result).await;
                    result
                })
                .await
                .clone()
        }
    }))
    .buffer_unordered(commands::MAX_CONCURRENT_FEED_REFRESHES);

    while refreshes.next().await.is_some() {}

    let classification = {
        let db = app_handle.state::<Db>();
        commands::classify_zone_without_refresh(
            &db,
            &ai_settings,
            workflow.zone.id,
            processing_limit,
        )
        .await
    };
    let snapshot = {
        let mut status = status.lock().await;
        match classification {
            Ok(result) => {
                status.articles_classified += result.articles_classified;
                status.articles_relevant += result.articles_relevant;
                status.articles_queued += result.articles_skipped_by_limit;
                // Articles held back by the burst cap are queued work too, and
                // are what the drain will pick up once the cycle finishes.
                status.articles_queued += result.articles_pending;
                status
                    .failures
                    .extend(
                        result
                            .failures
                            .into_iter()
                            .map(|failure| ClassifyZoneFailure {
                                source: workflow.zone.name.clone(),
                                message: failure.message,
                            }),
                    );
            }
            Err(err) => status.failures.push(ClassifyZoneFailure {
                source: workflow.zone.name,
                message: commands::compact_error(&err),
            }),
        }
        status.zones_completed += 1;
        status.clone()
    };
    emit_status(&app_handle, &snapshot);
}

async fn record_source_result(
    app_handle: &AppHandle,
    status: &AsyncMutex<AutoRefreshStatus>,
    feed: &Feed,
    result: &Result<FeedRefreshResult, String>,
) {
    let snapshot = {
        let mut status = status.lock().await;
        match result {
            Ok(result) => {
                status.sources_refreshed += 1;
                status.articles_fetched += result.fetched;
                status.articles_inserted += result.inserted;
            }
            Err(err) => status.failures.push(ClassifyZoneFailure {
                source: feed.title.clone(),
                message: commands::compact_error(err),
            }),
        }
        status.sources_completed += 1;
        status.clone()
    };
    emit_status(app_handle, &snapshot);
}

fn next_delay(db: &Db, interval_minutes: i64) -> Duration {
    let full_interval = Duration::from_secs((interval_minutes as u64) * 60);
    let Ok(Some(raw)) = db.get_setting(LAST_AUTO_REFRESH_AT_KEY) else {
        return full_interval;
    };
    let Ok(last_refresh) = DateTime::parse_from_rfc3339(&raw) else {
        return full_interval;
    };

    let elapsed_seconds = Utc::now()
        .signed_duration_since(last_refresh.with_timezone(&Utc))
        .num_seconds()
        .max(0) as u64;
    full_interval.saturating_sub(Duration::from_secs(elapsed_seconds))
}

fn fail_cycle(app_handle: &AppHandle, status: &mut AutoRefreshStatus, source: &str, error: String) {
    status.phase = "failed".to_string();
    status.completed_at = Some(Utc::now().to_rfc3339());
    status.failures.push(ClassifyZoneFailure {
        source: source.to_string(),
        message: commands::compact_error(&error),
    });
    emit_status(app_handle, status);
}

fn emit_status(app_handle: &AppHandle, status: &AutoRefreshStatus) {
    if let Err(err) = app_handle.emit(STATUS_EVENT, status.clone()) {
        eprintln!("Could not emit auto-refresh status: {err}");
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;

    use super::*;
    use crate::db::FeedMetadata;

    #[test]
    fn waits_full_interval_without_a_previous_attempt() {
        let db = Db::new(std::path::Path::new(":memory:")).expect("create in-memory db");

        assert_eq!(next_delay(&db, 30), Duration::from_secs(30 * 60));
    }

    #[test]
    fn runs_immediately_when_the_previous_attempt_is_overdue() {
        let db = Db::new(std::path::Path::new(":memory:")).expect("create in-memory db");
        let previous_attempt = (Utc::now() - ChronoDuration::minutes(10)).to_rfc3339();
        db.update_setting(LAST_AUTO_REFRESH_AT_KEY, &previous_attempt)
            .expect("save previous attempt");

        assert_eq!(next_delay(&db, 5), Duration::ZERO);
    }

    #[test]
    fn refresh_scope_contains_only_unique_zone_sources() {
        let db = Db::new(std::path::Path::new(":memory:")).expect("create in-memory db");
        let assigned = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/assigned.xml".to_string(),
                    title: "Assigned".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert assigned source");
        let unassigned = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/unassigned.xml".to_string(),
                    title: "Unassigned".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert unassigned source");
        let first_zone = db
            .create_zone("First", "first filter", "focused")
            .expect("create zone");
        let second_zone = db
            .create_zone("Second", "second filter", "focused")
            .expect("create zone");
        db.assign_feed(first_zone, assigned).expect("assign first");
        db.assign_feed(second_zone, assigned)
            .expect("assign shared");

        let workflows = load_zone_workflows(&db).expect("load workflows");

        assert_eq!(workflows.len(), 2);
        assert_eq!(unique_source_count(&workflows), 1);
        assert!(workflows
            .iter()
            .flat_map(|workflow| &workflow.feeds)
            .all(|feed| feed.id != unassigned));
    }
}
