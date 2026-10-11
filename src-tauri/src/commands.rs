use chrono::{DateTime, Duration, Utc};
use futures_util::{stream, StreamExt};
use tauri::{AppHandle, Manager, State};

use crate::ai::{self, AiModel, AiSettings, AiStatus, FilterBriefAnswer, FilterBriefAssistResult};
use crate::behavior;
use crate::db::{Article, ArticleOpenResult, Db, Feed, SmartFeedCounts, Zone};
use crate::feeds;
use crate::scheduler::SchedulerControl;

const DEFAULT_REFRESH_INTERVAL_MINUTES: i64 = 30;
const MIN_REFRESH_INTERVAL_MINUTES: i64 = 5;
const REFRESH_INTERVAL_KEY: &str = "refresh_interval_minutes";
const CLASSIFICATION_MODE_KEY: &str = "classification_processing_mode";
const CLASSIFICATION_LIMIT_KEY: &str = "classification_processing_limit";
const ARTICLE_RETENTION_DAYS_KEY: &str = "article_retention_days";
const LAST_ARTICLE_CLEANUP_AT_KEY: &str = "last_article_cleanup_at";
const ALL_UNCLASSIFIED_MODE: &str = "all_unclassified";
const LIMITED_MODE: &str = "limited";
const DEFAULT_CUSTOM_CLASSIFICATION_LIMIT: usize = 25;
const DEFAULT_ARTICLE_RETENTION_DAYS: i64 = 30;
const ARTICLE_CLEANUP_INTERVAL_HOURS: i64 = 24;
const MAX_CLASSIFICATION_LIMIT: usize = 100;
const ALL_UNCLASSIFIED_LIMIT: usize = 0;
const ALL_UNCLASSIFIED_HOURS: i64 = 24;
const MAX_RECOMMENDED_SOURCES: usize = 10;
pub const MAX_CONCURRENT_FEED_REFRESHES: usize = 4;
/// How many articles a single background pass classifies per Xone before
/// handing the remainder to the drain. Sized so the newest day of a busy Xone
/// appears within seconds of opening the app.
pub const COLD_START_BURST_LIMIT: usize = 50;
/// Failures are deduplicated before being reported: a provider that is down
/// fails identically for every article, and one line says as much as fifty.
const MAX_REPORTED_FAILURES: usize = 3;

#[derive(Debug, serde::Serialize)]
pub struct ClassifyZoneResult {
    pub classified: usize,
    pub relevant: usize,
    pub sources_refreshed: usize,
    pub articles_fetched: usize,
    pub articles_classified: usize,
    pub articles_relevant: usize,
    pub articles_skipped_by_limit: usize,
    /// Articles left unclassified because this pass hit its burst cap. They
    /// remain eligible and are picked up by the background drain.
    pub articles_pending: usize,
    pub processing_limit: usize,
    pub failures: Vec<ClassifyZoneFailure>,
}

/// One classification pass over a Xone's unclassified articles.
#[derive(Clone, Copy)]
pub struct ClassifyPass {
    pub reclassify: bool,
    pub processing_limit: Option<usize>,
    pub refresh_sources: bool,
    /// Cap on how many articles this pass classifies, independent of the saved
    /// coverage mode. Opening the app after a long absence would otherwise try
    /// to work through the entire 24-hour backlog of every Xone at once.
    pub burst_limit: Option<usize>,
}

impl ClassifyPass {
    /// A user-initiated run: no burst cap, because the user is watching and
    /// asked for this specific Xone.
    pub fn manual(
        reclassify: bool,
        processing_limit: Option<usize>,
        refresh_sources: bool,
    ) -> Self {
        Self {
            reclassify,
            processing_limit,
            refresh_sources,
            burst_limit: None,
        }
    }

    /// A background pass, capped so the newest articles land quickly and the
    /// rest is drained afterwards at a paced rate.
    pub fn background(processing_limit: usize, burst_limit: usize) -> Self {
        Self {
            reclassify: false,
            processing_limit: Some(processing_limit),
            refresh_sources: false,
            burst_limit: Some(burst_limit),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ClassifyZoneFailure {
    pub source: String,
    pub message: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    pub refresh_interval_minutes: i64,
    pub classification_processing_mode: String,
    pub classification_processing_limit: usize,
    pub article_retention_days: i64,
}

#[derive(Clone, Debug)]
pub struct FeedRefreshResult {
    pub fetched: usize,
    pub inserted: usize,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadContext {
    Zone { zone_id: i64 },
    SmartFeed,
    Other,
}

#[tauri::command]
pub fn list_feeds(db: State<'_, Db>) -> Result<Vec<Feed>, String> {
    db.list_feeds()
}

#[tauri::command]
pub async fn add_feed(
    url: String,
    display_name: Option<String>,
    category: String,
    region: Option<String>,
    db: State<'_, Db>,
) -> Result<Feed, String> {
    let display_name = display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if is_google_news_search_url(&url) && display_name.is_none() {
        return Err("Enter a publisher name for this Google News source".to_string());
    }
    let category = category.trim();
    if category.is_empty() {
        return Err("Choose or create a source category".to_string());
    }
    let region = region
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if category == "General News" && region.is_none() {
        return Err("Choose or create a source region".to_string());
    }
    let region = if category == "General News" {
        region
    } else {
        None
    };
    let mut fetched = feeds::fetch_feed(&url).await?;
    if let Some(display_name) = display_name {
        fetched.metadata.title = display_name.to_string();
    } else if let Some(saved_title) = db.feed_title(&url)? {
        // Re-adding an existing source must not replace its saved publisher
        // name with whatever title the feed currently serves.
        fetched.metadata.title = saved_title;
    }
    let feed_id = db.upsert_feed(&fetched.metadata, Some(category), region)?;
    let inserted = db.insert_articles(feed_id, &fetched.articles)?;
    if inserted > 0 {
        classify_zones_for_feed(&db, feed_id).await;
    }
    db.get_feed(feed_id)?
        .ok_or_else(|| "Feed was saved but could not be loaded".to_string())
}

fn is_google_news_search_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .is_ok_and(|url| url.host_str() == Some("news.google.com") && url.path() == "/rss/search")
}

#[tauri::command]
pub async fn refresh_feed(id: i64, db: State<'_, Db>) -> Result<usize, String> {
    refresh_feed_by_id(&db, id).await
}

pub async fn refresh_feed_by_id(db: &Db, id: i64) -> Result<usize, String> {
    let result = refresh_feed_without_classification(db, id).await?;
    if result.inserted > 0 {
        classify_zones_for_feed(db, id).await;
    }
    Ok(result.inserted)
}

pub async fn refresh_feed_without_classification(
    db: &Db,
    id: i64,
) -> Result<FeedRefreshResult, String> {
    let feed = db
        .get_feed(id)?
        .ok_or_else(|| "Feed not found".to_string())?;
    let validators = db.feed_cache_validators(id)?;
    let fetched = match feeds::fetch_feed_conditional(&feed.url, &validators).await? {
        feeds::FetchOutcome::NotModified => {
            // Nothing changed since the last poll: no body was sent, so there
            // is nothing to parse, dedup, or insert.
            db.touch_feed_fetch(id)?;
            return Ok(FeedRefreshResult {
                fetched: 0,
                inserted: 0,
            });
        }
        feeds::FetchOutcome::Fetched(fetched) => fetched,
    };
    let fetched_count = fetched.articles.len();
    db.update_feed_fetch(id, &fetched.metadata)?;
    let inserted = db.insert_articles(id, &fetched.articles)?;
    if !fetched.validators.is_empty() || !validators.is_empty() {
        db.save_feed_cache_validators(id, &fetched.validators)?;
    }
    Ok(FeedRefreshResult {
        fetched: fetched_count,
        inserted,
    })
}

#[tauri::command]
pub fn remove_feed(id: i64, db: State<'_, Db>) -> Result<(), String> {
    db.remove_feed(id)
}

#[tauri::command]
pub fn get_all_articles(feed_id: i64, db: State<'_, Db>) -> Result<Vec<Article>, String> {
    db.list_articles(feed_id)
}

#[tauri::command]
pub fn get_smart_feed_counts(db: State<'_, Db>) -> Result<SmartFeedCounts, String> {
    db.smart_feed_counts()
}

#[tauri::command]
pub fn get_today_articles(db: State<'_, Db>) -> Result<Vec<Article>, String> {
    db.list_today_articles()
}

#[tauri::command]
pub fn get_unread_articles(db: State<'_, Db>) -> Result<Vec<Article>, String> {
    db.list_unread_articles()
}

#[tauri::command]
pub fn list_zones(db: State<'_, Db>) -> Result<Vec<Zone>, String> {
    db.list_zones()
}

#[tauri::command]
pub fn create_zone(
    name: String,
    filter_description: String,
    focus_level: String,
    db: State<'_, Db>,
) -> Result<Zone, String> {
    let zone_id = db.create_zone(name.trim(), filter_description.trim(), focus_level.trim())?;
    db.get_zone(zone_id)?
        .ok_or_else(|| "Xone was saved but could not be loaded".to_string())
}

#[tauri::command]
pub fn update_zone(
    id: i64,
    name: String,
    filter_description: String,
    focus_level: String,
    db: State<'_, Db>,
) -> Result<Zone, String> {
    db.update_zone(
        id,
        name.trim(),
        filter_description.trim(),
        focus_level.trim(),
    )?;
    db.get_zone(id)?
        .ok_or_else(|| "Xone was updated but could not be loaded".to_string())
}

#[tauri::command]
pub fn delete_zone(id: i64, db: State<'_, Db>) -> Result<(), String> {
    db.delete_zone(id)
}

#[tauri::command]
pub fn reorder_zones(ordered_ids: Vec<i64>, db: State<'_, Db>) -> Result<(), String> {
    db.reorder_zones(&ordered_ids)
}

#[tauri::command]
pub fn assign_feed(zone_id: i64, feed_id: i64, db: State<'_, Db>) -> Result<(), String> {
    db.assign_feed(zone_id, feed_id)
}

#[tauri::command]
pub fn unassign_feed(zone_id: i64, feed_id: i64, db: State<'_, Db>) -> Result<(), String> {
    db.unassign_feed(zone_id, feed_id)
}

#[tauri::command]
pub fn get_zone_feeds(zone_id: i64, db: State<'_, Db>) -> Result<Vec<Feed>, String> {
    db.get_zone_feeds(zone_id)
}

#[tauri::command]
pub fn get_zone_articles(zone_id: i64, db: State<'_, Db>) -> Result<Vec<Article>, String> {
    db.list_zone_articles(zone_id)
}

#[tauri::command]
pub async fn mark_read(
    id: i64,
    context: ReadContext,
    app_handle: AppHandle,
) -> Result<ArticleOpenResult, String> {
    let (zone_id, smart_feed) = match context {
        ReadContext::Zone { zone_id } => (Some(zone_id), false),
        ReadContext::SmartFeed => (None, true),
        ReadContext::Other => (None, false),
    };
    behavior::note_article_activity(&app_handle);
    let db_app_handle = app_handle.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let db = db_app_handle.state::<Db>();
        db.record_article_open(id, zone_id, smart_feed, behavior::READS_PER_BATCH)
    })
    .await
    .map_err(|err| format!("Could not finish article-open work: {err}"))??;
    for zone_id in &result.ready_behavior_zone_ids {
        behavior::queue_zone(app_handle.clone(), *zone_id);
    }
    Ok(result)
}

#[tauri::command]
pub fn retry_zone_behavior(
    zone_id: i64,
    app_handle: AppHandle,
    db: State<'_, Db>,
) -> Result<(), String> {
    if db.get_zone(zone_id)?.is_none() {
        return Err("Xone not found".to_string());
    }
    behavior::queue_zone(app_handle, zone_id);
    Ok(())
}

#[tauri::command]
pub fn reset_zone_behavior(
    zone_id: i64,
    app_handle: AppHandle,
    db: State<'_, Db>,
) -> Result<Zone, String> {
    db.reset_zone_behavior(zone_id)?;
    behavior::emit_update(&app_handle, zone_id, "reset");
    db.get_zone(zone_id)?
        .ok_or_else(|| "Xone was reset but could not be loaded".to_string())
}

#[tauri::command]
pub fn get_article_cleanup_count(retention_days: i64, db: State<'_, Db>) -> Result<usize, String> {
    count_old_articles(&db, retention_days)
}

#[tauri::command]
pub fn cleanup_old_articles(retention_days: i64, db: State<'_, Db>) -> Result<usize, String> {
    let deleted = delete_old_articles(&db, retention_days)?;
    if deleted > 0 {
        if let Err(err) = db.compact() {
            eprintln!("Could not compact article storage after cleanup: {err}");
        }
    }
    if let Err(err) = db.update_setting(LAST_ARTICLE_CLEANUP_AT_KEY, &Utc::now().to_rfc3339()) {
        eprintln!("Could not save the article cleanup time: {err}");
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn get_app_settings(db: State<'_, Db>) -> Result<AppSettings, String> {
    Ok(AppSettings {
        refresh_interval_minutes: refresh_interval_minutes(&db)?,
        classification_processing_mode: classification_processing_mode(&db)?,
        classification_processing_limit: stored_classification_processing_limit(&db)?,
        article_retention_days: article_retention_days(&db)?,
    })
}

#[tauri::command]
pub async fn update_app_settings(
    settings: AppSettings,
    db: State<'_, Db>,
    scheduler: State<'_, SchedulerControl>,
) -> Result<AppSettings, String> {
    let interval = normalize_refresh_interval(settings.refresh_interval_minutes);
    let classification_mode =
        normalize_classification_mode(&settings.classification_processing_mode);
    let classification_limit =
        normalize_stored_classification_limit(settings.classification_processing_limit);
    let retention_days = normalize_article_retention_days(settings.article_retention_days);
    db.update_setting(REFRESH_INTERVAL_KEY, &interval.to_string())?;
    db.update_setting(CLASSIFICATION_MODE_KEY, classification_mode)?;
    db.update_setting(CLASSIFICATION_LIMIT_KEY, &classification_limit.to_string())?;
    db.update_setting(ARTICLE_RETENTION_DAYS_KEY, &retention_days.to_string())?;
    scheduler.reschedule();
    Ok(AppSettings {
        refresh_interval_minutes: interval,
        classification_processing_mode: classification_mode.to_string(),
        classification_processing_limit: classification_limit,
        article_retention_days: retention_days,
    })
}

#[tauri::command]
pub async fn get_ai_settings(db: State<'_, Db>) -> Result<AiSettings, String> {
    Ok(ai::settings_from_rows(db.get_settings()?))
}

#[tauri::command]
pub async fn update_ai_settings(
    settings: AiSettings,
    app_handle: AppHandle,
    db: State<'_, Db>,
) -> Result<AiSettings, String> {
    let settings = ai::normalize_settings(settings)?;
    db.update_settings(&ai::settings_to_rows(&settings))?;
    let saved = ai::settings_from_rows(db.get_settings()?);
    behavior::queue_ready(app_handle);
    Ok(saved)
}

#[tauri::command]
pub async fn get_ai_status(db: State<'_, Db>) -> Result<AiStatus, String> {
    let settings = ai::settings_from_rows(db.get_settings()?);
    Ok(ai::check_status(&settings).await)
}

#[tauri::command]
pub async fn check_ai_status(settings: AiSettings) -> Result<AiStatus, String> {
    Ok(ai::check_status(&settings).await)
}

#[tauri::command]
pub async fn fetch_ai_models(settings: AiSettings) -> Result<Vec<AiModel>, String> {
    ai::fetch_models(&settings).await
}

#[tauri::command]
pub async fn assist_filter_brief(
    filter_description: String,
    clarifications: Vec<FilterBriefAnswer>,
    db: State<'_, Db>,
) -> Result<FilterBriefAssistResult, String> {
    let settings = ai::settings_from_rows(db.get_settings()?);
    ai::assist_filter_brief(&settings, &filter_description, &clarifications).await
}

#[tauri::command]
pub async fn recommend_sources(
    filter_description: String,
    excluded_feed_ids: Vec<i64>,
    limit: usize,
    db: State<'_, Db>,
) -> Result<Vec<i64>, String> {
    let filter_description = filter_description.trim();
    if filter_description.is_empty() {
        return Err("Enter a Xone filter before requesting recommendations".to_string());
    }

    let limit = limit.min(MAX_RECOMMENDED_SOURCES);
    if limit == 0 {
        return Ok(Vec::new());
    }

    let settings = ai::settings_from_rows(db.get_settings()?);
    let feeds = db.list_feeds()?;
    ai::recommend_source_ids(
        &settings,
        filter_description,
        &feeds,
        &excluded_feed_ids,
        limit,
    )
    .await
}

#[tauri::command]
pub async fn classify_zone(
    zone_id: i64,
    reclassify: bool,
    processing_limit: Option<usize>,
    db: State<'_, Db>,
) -> Result<ClassifyZoneResult, String> {
    classify_zone_inner(&db, zone_id, reclassify, processing_limit, true).await
}

#[tauri::command]
pub fn open_in_browser(url: String) -> Result<(), String> {
    let url = crate::security::browser_url(&url)?;
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|err| err.to_string())
}

async fn classify_zones_for_feed(db: &Db, feed_id: i64) {
    let Ok(settings) = db.get_settings().map(ai::settings_from_rows) else {
        return;
    };
    let Ok(processing_limit) = classification_processing_limit(db) else {
        return;
    };
    if !ai::can_classify(&settings) {
        return;
    }

    let Ok(zones) = db.list_zones_for_feed(feed_id) else {
        return;
    };
    for zone in zones {
        let _ = classify_zone_with_settings(
            db,
            &settings,
            zone.id,
            ClassifyPass::background(processing_limit, COLD_START_BURST_LIMIT),
        )
        .await;
    }
}

pub fn refresh_interval_minutes(db: &Db) -> Result<i64, String> {
    let raw = db.get_setting(REFRESH_INTERVAL_KEY)?;
    let Some(raw) = raw else {
        return Ok(DEFAULT_REFRESH_INTERVAL_MINUTES);
    };

    let parsed = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| "Refresh interval setting is invalid".to_string())?;
    Ok(normalize_refresh_interval(parsed))
}

pub fn classification_processing_limit(db: &Db) -> Result<usize, String> {
    if classification_processing_mode(db)? == ALL_UNCLASSIFIED_MODE {
        return Ok(ALL_UNCLASSIFIED_LIMIT);
    }
    stored_classification_processing_limit(db)
}

pub fn article_retention_days(db: &Db) -> Result<i64, String> {
    let raw = db.get_setting(ARTICLE_RETENTION_DAYS_KEY)?;
    let Some(raw) = raw else {
        return Ok(DEFAULT_ARTICLE_RETENTION_DAYS);
    };
    let parsed = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| "Article retention setting is invalid".to_string())?;
    Ok(normalize_article_retention_days(parsed))
}

pub fn cleanup_old_articles_if_due(db: &Db) -> Result<usize, String> {
    if let Some(raw) = db.get_setting(LAST_ARTICLE_CLEANUP_AT_KEY)? {
        if let Ok(last_cleanup) = DateTime::parse_from_rfc3339(&raw) {
            let elapsed = Utc::now().signed_duration_since(last_cleanup.with_timezone(&Utc));
            if elapsed < Duration::hours(ARTICLE_CLEANUP_INTERVAL_HOURS) {
                return Ok(0);
            }
        }
    }

    let deleted = delete_old_articles(db, article_retention_days(db)?)?;
    db.update_setting(LAST_ARTICLE_CLEANUP_AT_KEY, &Utc::now().to_rfc3339())?;
    Ok(deleted)
}

fn count_old_articles(db: &Db, retention_days: i64) -> Result<usize, String> {
    let Some(cutoff) = article_retention_cutoff(retention_days) else {
        return Ok(0);
    };
    db.count_articles_older_than(&cutoff)
}

fn delete_old_articles(db: &Db, retention_days: i64) -> Result<usize, String> {
    let Some(cutoff) = article_retention_cutoff(retention_days) else {
        return Ok(0);
    };
    db.delete_articles_older_than(&cutoff)
}

fn article_retention_cutoff(retention_days: i64) -> Option<String> {
    let retention_days = normalize_article_retention_days(retention_days);
    if retention_days == 0 {
        return None;
    }
    Some((Utc::now() - Duration::days(retention_days)).to_rfc3339())
}

fn classification_processing_mode(db: &Db) -> Result<String, String> {
    let raw = db.get_setting(CLASSIFICATION_MODE_KEY)?;
    Ok(normalize_classification_mode(raw.as_deref().unwrap_or(ALL_UNCLASSIFIED_MODE)).to_string())
}

fn stored_classification_processing_limit(db: &Db) -> Result<usize, String> {
    let raw = db.get_setting(CLASSIFICATION_LIMIT_KEY)?;
    let Some(raw) = raw else {
        return Ok(DEFAULT_CUSTOM_CLASSIFICATION_LIMIT);
    };

    let parsed = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| "Classification processing limit setting is invalid".to_string())?;
    Ok(normalize_stored_classification_limit(parsed))
}

fn normalize_refresh_interval(minutes: i64) -> i64 {
    minutes.max(MIN_REFRESH_INTERVAL_MINUTES)
}

fn normalize_article_retention_days(days: i64) -> i64 {
    match days {
        0 | 7 | 14 | 30 | 90 => days,
        _ => DEFAULT_ARTICLE_RETENTION_DAYS,
    }
}

async fn classify_zone_inner(
    db: &Db,
    zone_id: i64,
    reclassify: bool,
    processing_limit: Option<usize>,
    refresh_sources: bool,
) -> Result<ClassifyZoneResult, String> {
    let settings = ai::settings_from_rows(db.get_settings()?);
    classify_zone_with_settings(
        db,
        &settings,
        zone_id,
        ClassifyPass::manual(reclassify, processing_limit, refresh_sources),
    )
    .await
}

pub async fn classify_zone_with_settings(
    db: &Db,
    settings: &AiSettings,
    zone_id: i64,
    pass: ClassifyPass,
) -> Result<ClassifyZoneResult, String> {
    let limit = normalize_classification_limit(pass.processing_limit);
    let zone = db
        .get_zone(zone_id)?
        .ok_or_else(|| "Xone not found".to_string())?;

    let mut result = ClassifyZoneResult {
        classified: 0,
        relevant: 0,
        sources_refreshed: 0,
        articles_fetched: 0,
        articles_classified: 0,
        articles_relevant: 0,
        articles_skipped_by_limit: 0,
        articles_pending: 0,
        processing_limit: limit,
        failures: Vec::new(),
    };

    if pass.refresh_sources {
        refresh_zone_sources(db, zone_id, &mut result).await?;
    }

    let cutoff = (limit == ALL_UNCLASSIFIED_LIMIT).then(|| {
        (chrono::Utc::now() - chrono::Duration::hours(ALL_UNCLASSIFIED_HOURS)).to_rfc3339()
    });
    let mut articles =
        db.list_zone_articles_for_classification(zone_id, pass.reclassify, cutoff.as_deref())?;
    if limit != ALL_UNCLASSIFIED_LIMIT && articles.len() > limit {
        result.articles_skipped_by_limit = articles.len() - limit;
        articles.truncate(limit);
    }
    // Articles arrive newest-first, so the burst cap keeps the most recent
    // ones and leaves the tail for the drain.
    if let Some(burst_limit) = pass.burst_limit {
        if articles.len() > burst_limit {
            result.articles_pending = articles.len() - burst_limit;
            articles.truncate(burst_limit);
        }
    }
    if articles.is_empty() {
        return Ok(result);
    }

    let attempted = articles.len();
    let filter_description = zone.filter_description.as_str();
    let behavior_profile = zone.behavior_profile.as_str();
    let concurrency = ai::max_concurrent_requests(settings);

    // Each classification is one HTTP round trip that the task otherwise
    // spends parked. Keeping a few in flight turns a sequential wait into a
    // bounded fan-out; the SQLite write behind each one is sub-millisecond.
    let outcomes = stream::iter(articles.into_iter().map(|article| async move {
        let outcome =
            ai::classify_article(settings, filter_description, behavior_profile, &article)
                .await
                .and_then(|classification| {
                    db.save_classification(
                        zone_id,
                        article.id,
                        classification.match_level.as_str(),
                    )?;
                    Ok(classification)
                });
        outcome
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await;

    for outcome in outcomes {
        match outcome {
            Ok(classification) => {
                result.classified += 1;
                result.articles_classified += 1;
                if classification.match_level.is_visible_at(&zone.focus_level) {
                    result.relevant += 1;
                    result.articles_relevant += 1;
                }
            }
            Err(err) => push_unique_failure(&mut result.failures, "AI classification", &err),
        }
    }

    // Nothing succeeded, so the provider is unreachable or misconfigured.
    // Draining the remainder would only repeat the same failure.
    if result.articles_classified == 0 && attempted > 0 {
        result.articles_pending = 0;
    }

    Ok(result)
}

/// Report a failure once per distinct message. A provider that is down fails
/// identically for every article in the pass.
fn push_unique_failure(failures: &mut Vec<ClassifyZoneFailure>, source: &str, error: &str) {
    let message = compact_error(error);
    if failures.len() >= MAX_REPORTED_FAILURES
        || failures.iter().any(|failure| failure.message == message)
    {
        return;
    }
    failures.push(ClassifyZoneFailure {
        source: source.to_string(),
        message,
    });
}

pub async fn classify_zone_without_refresh(
    db: &Db,
    settings: &AiSettings,
    zone_id: i64,
    processing_limit: usize,
) -> Result<ClassifyZoneResult, String> {
    classify_zone_with_settings(
        db,
        settings,
        zone_id,
        ClassifyPass::background(processing_limit, COLD_START_BURST_LIMIT),
    )
    .await
}

async fn refresh_zone_sources(
    db: &Db,
    zone_id: i64,
    result: &mut ClassifyZoneResult,
) -> Result<(), String> {
    let feeds = db.get_zone_feeds(zone_id)?;
    let mut refreshes = stream::iter(feeds.into_iter().map(|feed| async move {
        let refresh = refresh_feed_without_classification(db, feed.id).await;
        (feed, refresh)
    }))
    .buffer_unordered(MAX_CONCURRENT_FEED_REFRESHES);

    while let Some((feed, refresh)) = refreshes.next().await {
        match refresh {
            Ok(refresh) => {
                result.sources_refreshed += 1;
                result.articles_fetched += refresh.fetched;
            }
            Err(err) => result.failures.push(ClassifyZoneFailure {
                source: feed.title,
                message: compact_error(&err),
            }),
        }
    }

    Ok(())
}

fn normalize_classification_limit(limit: Option<usize>) -> usize {
    match limit {
        Some(ALL_UNCLASSIFIED_LIMIT) => ALL_UNCLASSIFIED_LIMIT,
        Some(limit) => normalize_stored_classification_limit(limit),
        None => ALL_UNCLASSIFIED_LIMIT,
    }
}

fn normalize_stored_classification_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_CLASSIFICATION_LIMIT)
}

fn normalize_classification_mode(mode: &str) -> &'static str {
    if mode == LIMITED_MODE {
        LIMITED_MODE
    } else {
        ALL_UNCLASSIFIED_MODE
    }
}

pub fn compact_error(message: &str) -> String {
    let single_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_LEN: usize = 180;
    if single_line.chars().count() <= MAX_LEN {
        single_line
    } else {
        format!(
            "{}...",
            single_line.chars().take(MAX_LEN).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_old_articles_if_due, is_google_news_search_url, normalize_article_retention_days,
        normalize_classification_limit, normalize_classification_mode, ALL_UNCLASSIFIED_LIMIT,
        ALL_UNCLASSIFIED_MODE, DEFAULT_ARTICLE_RETENTION_DAYS, LIMITED_MODE,
    };
    use super::{push_unique_failure, ClassifyPass, COLD_START_BURST_LIMIT, MAX_REPORTED_FAILURES};
    use crate::db::{Db, FeedCacheValidators, FeedMetadata, NewArticle};
    use chrono::{Duration, Utc};
    use std::path::Path;

    #[test]
    fn cache_validators_round_trip_and_a_not_modified_poll_only_moves_the_fetch_time() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/conditional.xml".to_string(),
                    title: "Conditional".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");

        assert!(
            db.feed_cache_validators(feed_id)
                .expect("read validators")
                .is_empty(),
            "a source that has never been fetched must send an unconditional request"
        );

        db.save_feed_cache_validators(
            feed_id,
            &FeedCacheValidators {
                etag: Some("\"a3f5c9\"".to_string()),
                last_modified: Some("Fri, 05 Sep 2026 14:22:00 GMT".to_string()),
            },
        )
        .expect("save validators");

        let saved = db.feed_cache_validators(feed_id).expect("read validators");
        assert_eq!(saved.etag.as_deref(), Some("\"a3f5c9\""));
        assert_eq!(
            saved.last_modified.as_deref(),
            Some("Fri, 05 Sep 2026 14:22:00 GMT")
        );

        // A 304 carries no metadata, so only the fetch time may change.
        db.touch_feed_fetch(feed_id).expect("touch fetch time");
        let feed = db.get_feed(feed_id).expect("load feed").expect("feed row");
        assert!(feed.last_fetched_at.is_some());
        assert_eq!(feed.title, "Conditional");
        assert!(!db
            .feed_cache_validators(feed_id)
            .expect("read validators")
            .is_empty());
    }

    #[test]
    fn a_failing_provider_is_reported_once_rather_than_per_article() {
        let mut failures = Vec::new();
        for _ in 0..50 {
            push_unique_failure(&mut failures, "AI classification", "Could not reach Ollama");
        }
        assert_eq!(failures.len(), 1);

        push_unique_failure(&mut failures, "AI classification", "Invalid model");
        assert_eq!(failures.len(), 2);
        for index in 0..10 {
            push_unique_failure(
                &mut failures,
                "AI classification",
                &format!("error {index}"),
            );
        }
        assert_eq!(failures.len(), MAX_REPORTED_FAILURES);
    }

    #[test]
    fn only_background_passes_cap_the_burst() {
        assert_eq!(ClassifyPass::manual(false, None, true).burst_limit, None);
        assert_eq!(
            ClassifyPass::background(0, COLD_START_BURST_LIMIT).burst_limit,
            Some(COLD_START_BURST_LIMIT)
        );
    }

    #[test]
    fn only_google_news_search_urls_require_a_display_name() {
        assert!(is_google_news_search_url(
            "https://news.google.com/rss/search?q=site:example.com+when:1d"
        ));
        assert!(!is_google_news_search_url(
            "https://news.google.com/rss/topics/example"
        ));
        assert!(!is_google_news_search_url(
            "https://example.com/rss/search?q=site:example.com"
        ));
    }

    #[test]
    fn all_unclassified_mode_preserves_the_unlimited_sentinel() {
        assert_eq!(normalize_classification_limit(None), ALL_UNCLASSIFIED_LIMIT);
        assert_eq!(
            normalize_classification_limit(Some(ALL_UNCLASSIFIED_LIMIT)),
            ALL_UNCLASSIFIED_LIMIT
        );
        assert_eq!(
            normalize_classification_mode("unknown"),
            ALL_UNCLASSIFIED_MODE
        );
        assert_eq!(normalize_classification_mode(LIMITED_MODE), LIMITED_MODE);
    }

    #[test]
    fn article_retention_accepts_only_supported_periods() {
        for days in [0, 7, 14, 30, 90] {
            assert_eq!(normalize_article_retention_days(days), days);
        }
        assert_eq!(
            normalize_article_retention_days(365),
            DEFAULT_ARTICLE_RETENTION_DAYS
        );
    }

    #[test]
    fn automatic_article_cleanup_runs_at_most_once_per_day() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/daily-cleanup.xml".to_string(),
                    title: "Daily Cleanup".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");
        let old_published_at = (Utc::now() - Duration::days(40)).to_rfc3339();
        db.insert_articles(
            feed_id,
            &[NewArticle {
                title: "First expired daily cleanup story".to_string(),
                content: "Old content".to_string(),
                summary: None,
                url: "https://example.com/first-expired-cleanup".to_string(),
                author: None,
                published_at: Some(old_published_at.clone()),
            }],
        )
        .expect("insert first expired article");

        assert_eq!(
            cleanup_old_articles_if_due(&db).expect("run first cleanup"),
            1
        );

        db.insert_articles(
            feed_id,
            &[NewArticle {
                title: "Second expired archive item".to_string(),
                content: "Old content".to_string(),
                summary: None,
                url: "https://example.com/second-expired-cleanup".to_string(),
                author: None,
                published_at: Some(old_published_at),
            }],
        )
        .expect("insert second expired article");

        assert_eq!(
            cleanup_old_articles_if_due(&db).expect("skip second cleanup"),
            0
        );
        assert_eq!(db.list_articles(feed_id).expect("list articles").len(), 1);
    }
}
