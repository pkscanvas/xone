//! Cached, evidence-backed rolling briefings. All AI work belongs to one worker.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;

use crate::{
    ai,
    db::{Article, Db, Zone},
};

const CADENCE_SECONDS: i64 = 120;
const RETRY_SECONDS: i64 = 600;
const PAYLOAD_LIMIT: usize = 16_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sentence {
    pub text: String,
    pub article_ids: Vec<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Briefing {
    sentences: Vec<Sentence>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Cache {
    fingerprint: String,
    criteria: String,
    evidence: BTreeMap<i64, String>,
    sentences: Vec<Sentence>,
    generated_at: String,
    window_start: String,
    window_end: String,
}

struct Snapshot {
    zone: Zone,
    articles: Vec<Article>,
    evidence: BTreeMap<i64, String>,
    criteria: String,
    fingerprint: String,
    now: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct TldrEntry {
    zone_id: i64,
    sentences: Vec<Sentence>,
    citations: Vec<Article>,
    generated_at: Option<String>,
    window_start: Option<String>,
    window_end: Option<String>,
    article_count: usize,
    status: &'static str,
    stale: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Attempt {
    started_at: Option<DateTime<Utc>>,
    failed_at: Option<DateTime<Utc>>,
    error: Option<String>,
    running: bool,
}

impl Attempt {
    fn due(&self, now: DateTime<Utc>, force: bool, unchanged: bool) -> bool {
        if self.running {
            return false;
        }
        if force {
            return true;
        }
        if unchanged && self.error.is_none() {
            return false;
        }
        if self
            .failed_at
            .is_some_and(|at| (now - at).num_seconds() < RETRY_SECONDS)
        {
            return false;
        }
        !self
            .started_at
            .is_some_and(|at| (now - at).num_seconds() < CADENCE_SECONDS)
    }
}

#[derive(Default)]
struct WorkState {
    attempts: HashMap<i64, Attempt>,
    forced: BTreeSet<i64>,
}

#[derive(Default)]
pub struct TldrControl {
    state: Mutex<WorkState>,
    wake: Notify,
}

fn fingerprint(value: &impl Serialize) -> Result<String, String> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(value)
        .map_err(|err| err.to_string())?
        .hash(&mut hash);
    Ok(format!("{:016x}", hash.finish()))
}

fn in_window(article: &Article, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(
        article
            .published_at
            .as_deref()
            .unwrap_or(&article.fetched_at),
    )
    .map(|at| at <= now && at > now - chrono::Duration::hours(24))
    .unwrap_or(false)
}

fn snapshot(
    db: &Db,
    zone_id: i64,
    settings: &ai::AiSettings,
    now: DateTime<Utc>,
) -> Result<Snapshot, String> {
    let zone = db.get_zone(zone_id)?.ok_or("Xone no longer exists")?;
    let articles: Vec<_> = db
        .list_zone_articles(zone_id)?
        .into_iter()
        .filter(|article| in_window(article, now))
        .collect();
    let mut feeds: Vec<_> = db
        .get_zone_feeds(zone_id)?
        .iter()
        .map(|feed| feed.id)
        .collect();
    feeds.sort_unstable();
    // Only the active provider matters. Neither credentials nor article read state
    // are written to the cache. The version invalidates old prompt contracts.
    let criteria = fingerprint(&(
        1,
        &zone.filter_description,
        &zone.focus_level,
        feeds,
        &settings.provider,
        settings.providers.get(&settings.provider),
    ))?;
    let evidence = articles
        .iter()
        .map(|article| {
            fingerprint(&(
                article.id,
                &article.title,
                &article.summary,
                &article.content,
                &article.url,
                &article.published_at,
                &article.fetched_at,
            ))
            .map(|hash| (article.id, hash))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let fingerprint = fingerprint(&(&criteria, &evidence))?;
    Ok(Snapshot {
        zone,
        articles,
        evidence,
        criteria,
        fingerprint,
        now,
    })
}

fn load_cache(db: &Db, id: i64) -> Result<Option<Cache>, String> {
    // A damaged/older cache is expendable: regenerate instead of blocking news.
    Ok(db
        .get_zone_summary(id)?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn cache_safe(cache: &Cache, current: &Snapshot) -> bool {
    cache.criteria == current.criteria
        && cache
            .evidence
            .iter()
            .all(|(id, hash)| current.evidence.get(id) == Some(hash))
}

fn entry(
    current: &Snapshot,
    cache: Option<Cache>,
    attempt: Option<&Attempt>,
    configured: bool,
) -> TldrEntry {
    let safe = cache.filter(|cache| cache_safe(cache, current));
    let stale = safe
        .as_ref()
        .is_none_or(|cache| cache.fingerprint != current.fingerprint);
    let error = attempt.and_then(|attempt| attempt.error.clone());
    let running = attempt.is_some_and(|attempt| attempt.running);
    let status = if current.articles.is_empty() {
        "empty"
    } else if !configured {
        "unconfigured"
    } else if running {
        "updating"
    } else if error.is_some() {
        "error"
    } else if stale {
        "updating"
    } else {
        "ready"
    };
    let sentences = safe
        .as_ref()
        .map(|cache| cache.sentences.clone())
        .unwrap_or_default();
    let ids: BTreeSet<_> = sentences
        .iter()
        .flat_map(|sentence| &sentence.article_ids)
        .collect();
    let citations = current
        .articles
        .iter()
        .filter(|article| ids.contains(&article.id))
        .cloned()
        .collect();
    TldrEntry {
        zone_id: current.zone.id,
        sentences,
        citations,
        generated_at: safe.as_ref().map(|cache| cache.generated_at.clone()),
        window_start: safe.as_ref().map(|cache| cache.window_start.clone()),
        window_end: safe.as_ref().map(|cache| cache.window_end.clone()),
        article_count: current.articles.len(),
        status,
        stale: stale && !current.articles.is_empty(),
        error,
    }
}

#[tauri::command]
pub fn get_tldr(
    db: State<'_, Db>,
    control: State<'_, TldrControl>,
) -> Result<Vec<TldrEntry>, String> {
    let settings = ai::settings_from_rows(db.get_settings()?);
    let now = Utc::now();
    db.list_zones()?
        .into_iter()
        .map(|zone| {
            let current = snapshot(&db, zone.id, &settings, now)?;
            let cache = load_cache(&db, zone.id)?;
            let state = control.state.lock().map_err(|err| err.to_string())?;
            Ok(entry(
                &current,
                cache,
                state.attempts.get(&zone.id),
                ai::can_classify(&settings),
            ))
        })
        .collect()
}

#[tauri::command]
pub fn regenerate_tldr(
    zone_id: Option<i64>,
    db: State<'_, Db>,
    control: State<'_, TldrControl>,
) -> Result<(), String> {
    let ids: BTreeSet<_> = db.list_zones()?.iter().map(|zone| zone.id).collect();
    if zone_id.is_some_and(|id| !ids.contains(&id)) {
        return Err("Xone no longer exists".into());
    }
    let mut state = control.state.lock().map_err(|err| err.to_string())?;
    for id in ids
        .into_iter()
        .filter(|id| zone_id.is_none_or(|selected| *id == selected))
    {
        // Clicking repeatedly during a generation must not queue duplicate work.
        if !state
            .attempts
            .get(&id)
            .is_some_and(|attempt| attempt.running)
        {
            state.forced.insert(id);
        }
    }
    drop(state);
    control.wake.notify_one();
    Ok(())
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(CADENCE_SECONDS as u64));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let control = app.state::<TldrControl>();
            tokio::select! { _ = timer.tick() => {}, _ = control.wake.notified() => {} }
            if let Err(err) = run_pass(&app).await {
                eprintln!("Could not update TL:DR: {err}");
            }
        }
    });
}

fn emit(app: &AppHandle, zone_id: i64) {
    let _ = app.emit("tldr-updated", zone_id);
}

async fn run_pass(app: &AppHandle) -> Result<(), String> {
    let db = app.state::<Db>();
    let control = app.state::<TldrControl>();
    let zones = db.list_zones()?;
    {
        let ids: BTreeSet<_> = zones.iter().map(|zone| zone.id).collect();
        let mut state = control.state.lock().map_err(|err| err.to_string())?;
        state.attempts.retain(|id, _| ids.contains(id));
        state.forced.retain(|id| ids.contains(id));
    }
    for zone in zones {
        let settings = ai::settings_from_rows(db.get_settings()?);
        let current = match snapshot(&db, zone.id, &settings, Utc::now()) {
            Ok(current) => current,
            Err(_) if db.get_zone(zone.id)?.is_none() => continue,
            Err(err) => return Err(err),
        };
        let cached = load_cache(&db, zone.id)?;
        let force = control
            .state
            .lock()
            .map_err(|err| err.to_string())?
            .forced
            .remove(&zone.id);
        let unchanged = cached
            .as_ref()
            .is_some_and(|cache| cache.fingerprint == current.fingerprint);
        // Notify even without AI work: an expired cache must disappear from an open page.
        emit(app, zone.id);
        if !current.articles.is_empty() && !ai::can_classify(&settings) {
            continue;
        }
        {
            let mut state = control.state.lock().map_err(|err| err.to_string())?;
            let attempt = state.attempts.entry(zone.id).or_default();
            if !attempt.due(Utc::now(), force, unchanged) {
                continue;
            }
            attempt.running = true;
            attempt.started_at = Some(Utc::now());
            attempt.error = None;
        }
        emit(app, zone.id);
        let result = if current.articles.is_empty() {
            Ok(vec![])
        } else {
            summarize(&settings, &current).await
        };
        // Re-read configuration and evidence after network I/O. Never resurrect a
        // deleted Xone or publish a result for old focus/assignment/provider settings.
        let latest = db
            .get_settings()
            .and_then(|rows| snapshot(&db, zone.id, &ai::settings_from_rows(rows), Utc::now()));
        let result = match (result, latest) {
            (Ok(sentences), Ok(latest)) => {
                let cache = Cache {
                    fingerprint: current.fingerprint,
                    criteria: current.criteria,
                    evidence: current.evidence,
                    sentences,
                    generated_at: Utc::now().to_rfc3339(),
                    window_start: (current.now - chrono::Duration::hours(24)).to_rfc3339(),
                    window_end: current.now.to_rfc3339(),
                };
                if cache_safe(&cache, &latest) {
                    db.save_zone_summary(
                        zone.id,
                        &serde_json::to_string(&cache).map_err(|err| err.to_string())?,
                    )
                } else {
                    Ok(())
                }
            }
            (_, Err(_)) => Ok(()), // deleted/edited during the request; retry a fresh snapshot next pass
            (Err(err), _) => Err(err),
        };
        {
            let mut state = control.state.lock().map_err(|err| err.to_string())?;
            let attempt = state.attempts.entry(zone.id).or_default();
            attempt.running = false;
            attempt.failed_at = result.as_ref().err().map(|_| Utc::now());
            // Provider details may contain private response content; keep the visible error compact.
            attempt.error = result
                .err()
                .map(|_| "Could not update this briefing. Try again.".into());
        }
        emit(app, zone.id);
    }
    Ok(())
}

#[derive(Clone, Serialize)]
struct Evidence {
    text: String,
    article_ids: Vec<i64>,
}

fn batches(evidence: Vec<Evidence>) -> Result<Vec<Vec<Evidence>>, String> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut size = 2; // JSON array brackets
    for item in evidence {
        let length = serde_json::to_string(&item)
            .map_err(|err| err.to_string())?
            .chars()
            .count();
        if length + 2 > PAYLOAD_LIMIT {
            return Err("An evidence item exceeds the summary limit".into());
        }
        if size + length + usize::from(!batch.is_empty()) > PAYLOAD_LIMIT {
            batches.push(std::mem::take(&mut batch));
            size = 2;
        }
        size += length + usize::from(!batch.is_empty());
        batch.push(item);
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

fn parse_briefing(raw: &str, allowed: &BTreeSet<i64>) -> Result<Vec<Sentence>, String> {
    let briefing: Briefing = serde_json::from_str(raw).map_err(|_| "Invalid briefing JSON")?;
    if briefing.sentences.is_empty() || briefing.sentences.len() > 3 {
        return Err("Expected 1-3 sentences".into());
    }
    for sentence in &briefing.sentences {
        if sentence.text.trim().is_empty()
            || sentence.text.chars().count() > 1200
            || sentence.article_ids.is_empty()
            || sentence.article_ids.len() > 4
            || sentence.article_ids.iter().any(|id| !allowed.contains(id))
        {
            return Err("Invalid briefing text or citation".into());
        }
    }
    Ok(briefing.sentences)
}

async fn summarize(settings: &ai::AiSettings, current: &Snapshot) -> Result<Vec<Sentence>, String> {
    summarize_with(current, |payload| async move {
        ai::summarize_news(settings, &current.zone.filter_description, &payload).await
    })
    .await
}

async fn summarize_with<F, Fut>(
    current: &Snapshot,
    mut generate: F,
) -> Result<Vec<Sentence>, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let mut evidence: Vec<_> = current
        .articles
        .iter()
        .map(|article| Evidence {
            text: format!(
                "{}\n{}",
                article.title.chars().take(1000).collect::<String>(),
                article
                    .summary
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or(&article.content)
                    .chars()
                    .take(400)
                    .collect::<String>()
            ),
            article_ids: vec![article.id],
        })
        .collect();
    loop {
        let groups = batches(evidence)?;
        let is_final = groups.len() == 1;
        let previous_count = groups.len();
        let mut reduced = Vec::new();
        for group in groups {
            let allowed = group
                .iter()
                .flat_map(|item| item.article_ids.iter().copied())
                .collect();
            let payload = serde_json::to_string(&group).map_err(|err| err.to_string())?;
            let raw = generate(payload).await?;
            let sentences = parse_briefing(&raw, &allowed)?;
            if is_final {
                return Ok(sentences);
            }
            reduced.extend(sentences.into_iter().map(|sentence| Evidence {
                text: sentence.text,
                article_ids: sentence.article_ids,
            }));
        }
        // A malformed verbose model must not cause an infinite reduction loop.
        if batches(reduced.clone())?.len() >= previous_count {
            return Err("Briefing did not fit the summary limit".into());
        }
        evidence = reduced;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{FeedMetadata, NewArticle};
    use std::path::Path;

    fn fixture() -> (Db, i64, i64, ai::AiSettings, DateTime<Utc>) {
        let db = Db::new(Path::new(":memory:")).unwrap();
        let feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://tldr.test/rss".into(),
                    title: "TLDR fixture".into(),
                    description: None,
                    site_url: None,
                },
                None,
                None,
            )
            .unwrap();
        let zone = db
            .create_zone("Science", "Research developments", "focused")
            .unwrap();
        db.assign_feed(zone, feed).unwrap();
        let now = Utc::now() + chrono::Duration::seconds(1);
        let settings = ai::settings_from_rows(db.get_settings().unwrap());
        (db, feed, zone, settings, now)
    }

    fn add(
        db: &Db,
        feed: i64,
        zone: i64,
        at: Option<DateTime<Utc>>,
        title: &str,
        level: &str,
    ) -> i64 {
        let url = format!("https://tldr.test/{}", title.replace(' ', "-"));
        db.insert_articles(
            feed,
            &[NewArticle {
                title: title.into(),
                content: "Reported evidence.".into(),
                summary: None,
                url: url.clone(),
                author: None,
                published_at: at.map(|at| at.to_rfc3339()),
            }],
        )
        .unwrap();
        let id = db
            .list_articles(feed)
            .unwrap()
            .into_iter()
            .find(|a| a.url == url)
            .unwrap()
            .id;
        db.save_classification(zone, id, level).unwrap();
        id
    }

    fn cache(current: &Snapshot) -> Cache {
        Cache {
            fingerprint: current.fingerprint.clone(),
            criteria: current.criteria.clone(),
            evidence: current.evidence.clone(),
            sentences: vec![Sentence {
                text: "A reported development.".into(),
                article_ids: vec![current.articles[0].id],
            }],
            generated_at: current.now.to_rfc3339(),
            window_start: (current.now - chrono::Duration::hours(24)).to_rfc3339(),
            window_end: current.now.to_rfc3339(),
        }
    }

    #[test]
    fn rolling_window_uses_publication_then_fetch_time_and_excludes_future() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now - chrono::Duration::hours(24)),
            "Expired boundary astronomy observation",
            "high",
        );
        add(
            &db,
            feed,
            zone,
            Some(now - chrono::Duration::hours(23)),
            "Recent ocean exploration findings",
            "high",
        );
        add(
            &db,
            feed,
            zone,
            Some(now + chrono::Duration::hours(1)),
            "Future fusion reactor experiment",
            "high",
        );
        add(
            &db,
            feed,
            zone,
            None,
            "Undated rainforest species discovery",
            "high",
        );
        let current = snapshot(&db, zone, &settings, now).unwrap();
        assert_eq!(current.articles.len(), 2);
        assert!(current
            .articles
            .iter()
            .all(|a| a.title.starts_with("Recent") || a.title.starts_with("Undated")));
    }

    #[test]
    fn focus_membership_and_read_independence_match_the_xone() {
        let (db, feed, zone, settings, now) = fixture();
        let high = add(
            &db,
            feed,
            zone,
            Some(now),
            "Satellite explores distant planetary atmosphere",
            "high",
        );
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Biologists identify previously unknown organisms",
            "medium",
        );
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Quantum laboratory publishes processor benchmark",
            "low",
        );
        let before = snapshot(&db, zone, &settings, now).unwrap();
        assert_eq!(before.articles.len(), 2);
        let second = db.create_zone("Second", "Research", "focused").unwrap();
        db.assign_feed(second, feed).unwrap();
        db.save_classification(second, high, "high").unwrap();
        db.record_article_open(high, Some(zone), false, 10).unwrap();
        let after = snapshot(&db, zone, &settings, now + chrono::Duration::minutes(1)).unwrap();
        assert_eq!(before.fingerprint, after.fingerprint);
        assert_eq!(
            db.get_zone(zone).unwrap().unwrap().behavior_reads_pending,
            1
        );
        assert_eq!(
            db.get_zone(second).unwrap().unwrap().behavior_reads_pending,
            0
        );
        db.update_zone(zone, "Science", "Research developments", "broad")
            .unwrap();
        assert_eq!(
            snapshot(&db, zone, &settings, now).unwrap().articles.len(),
            3
        );
        db.update_zone(zone, "Science", "Research developments", "laser")
            .unwrap();
        assert_eq!(
            snapshot(&db, zone, &settings, now).unwrap().articles.len(),
            1
        );
        db.unassign_feed(zone, feed).unwrap();
        assert!(snapshot(&db, zone, &settings, now)
            .unwrap()
            .articles
            .is_empty());
    }

    #[test]
    fn cached_result_survives_additions_but_not_removed_or_expired_evidence() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now - chrono::Duration::hours(23)),
            "Astronomers discover unusual rocky planet",
            "high",
        );
        let old = cache(&snapshot(&db, zone, &settings, now).unwrap());
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Researchers announce breakthrough cancer treatment",
            "high",
        );
        let newer = snapshot(&db, zone, &settings, now).unwrap();
        assert!(cache_safe(&old, &newer));
        let displayed = entry(&newer, Some(old.clone()), None, true);
        assert_eq!(displayed.status, "updating");
        assert!(displayed.stale && !displayed.sentences.is_empty());
        let expired = snapshot(&db, zone, &settings, now + chrono::Duration::hours(1)).unwrap();
        assert!(!cache_safe(&old, &expired));
        assert!(entry(&expired, Some(old), None, true).sentences.is_empty());
    }

    #[test]
    fn criteria_provider_and_deleted_xones_invalidate_results() {
        let (db, feed, zone, mut settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Ocean expedition maps hydrothermal vents",
            "high",
        );
        let old = cache(&snapshot(&db, zone, &settings, now).unwrap());
        settings
            .providers
            .get_mut(&settings.provider)
            .unwrap()
            .model = "another-model".into();
        assert!(!cache_safe(
            &old,
            &snapshot(&db, zone, &settings, now).unwrap()
        ));
        let settings = ai::settings_from_rows(db.get_settings().unwrap());
        db.update_zone(zone, "Science", "Only astronomy", "focused")
            .unwrap();
        assert!(!cache_safe(
            &old,
            &snapshot(&db, zone, &settings, now).unwrap()
        ));
        db.save_zone_summary(zone, &serde_json::to_string(&old).unwrap())
            .unwrap();
        assert!(load_cache(&db, zone).unwrap().is_some());
        db.delete_zone(zone).unwrap();
        assert!(db.get_zone_summary(zone).unwrap().is_none());
        assert!(snapshot(&db, zone, &settings, now).is_err());
    }

    #[test]
    fn clustered_reports_have_one_canonical_summary_input() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Scientists discover water beneath distant lunar crater",
            "high",
        );
        let second = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://tldr.test/other-rss".into(),
                    title: "Second publisher".into(),
                    description: None,
                    site_url: None,
                },
                None,
                None,
            )
            .unwrap();
        db.assign_feed(zone, second).unwrap();
        db.insert_articles(
            second,
            &[NewArticle {
                title: "Scientists discover water beneath distant lunar crater".into(),
                content: "Reported evidence.".into(),
                summary: None,
                url: "https://tldr.test/alternate-report".into(),
                author: None,
                published_at: Some(now.to_rfc3339()),
            }],
        )
        .unwrap();
        let current = snapshot(&db, zone, &settings, now).unwrap();
        assert_eq!(current.articles.len(), 1);
        assert_eq!(current.articles[0].related_articles.len(), 1);
    }

    #[test]
    fn cache_round_trip_reuses_unchanged_inputs_and_recovers_from_corruption() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Scientists measure magnetic field around distant star",
            "high",
        );
        let current = snapshot(&db, zone, &settings, now).unwrap();
        let old = cache(&current);
        db.save_zone_summary(zone, &serde_json::to_string(&old).unwrap())
            .unwrap();
        let restored = load_cache(&db, zone).unwrap().unwrap();
        assert_eq!(restored.fingerprint, current.fingerprint);
        assert_eq!(entry(&current, Some(restored), None, true).status, "ready");
        db.save_zone_summary(zone, "damaged cache").unwrap();
        assert!(load_cache(&db, zone).unwrap().is_none());
        assert_eq!(entry(&current, None, None, true).status, "updating");
    }

    #[test]
    fn malformed_or_unbacked_citations_are_rejected() {
        let allowed = BTreeSet::from([1, 2]);
        for raw in [
            "not json",
            r#"{"sentences":[]}"#,
            r#"{"sentences":[{"text":"Claim","article_ids":[99]}]}"#,
            r#"{"sentences":[{"text":"Claim","article_ids":[]}]}"#,
            r#"{"sentences":[{"text":"","article_ids":[1]}]}"#,
            r#"{"sentences":[{"text":"Claim","article_ids":[1],"url":"invented"}]}"#,
        ] {
            assert!(parse_briefing(raw, &allowed).is_err(), "{raw}");
        }
        assert!(parse_briefing(
            r#"{"sentences":[{"text":"Reported fact.","article_ids":[1,2]}]}"#,
            &allowed
        )
        .is_ok());
    }

    #[test]
    fn payload_batches_preserve_every_input_and_bound_escaped_unicode_json() {
        let evidence: Vec<_> = (0..200)
            .map(|id| Evidence {
                text: "\"研究\\\n".repeat(180),
                article_ids: vec![id],
            })
            .collect();
        let groups = batches(evidence).unwrap();
        assert!(groups.len() > 1);
        let mut ids = Vec::new();
        for group in groups {
            assert!(serde_json::to_string(&group).unwrap().chars().count() <= PAYLOAD_LIMIT);
            ids.extend(group.into_iter().flat_map(|item| item.article_ids));
        }
        assert_eq!(ids, (0..200).collect::<Vec<_>>());
    }

    #[test]
    fn worker_cadence_failure_backoff_and_inflight_dedup_use_a_controlled_clock() {
        let now = DateTime::parse_from_rfc3339("2026-10-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut attempt = Attempt::default();
        assert!(attempt.due(now, false, false)); // startup
        assert!(!attempt.due(now, false, true)); // persisted cache survives restart
        assert!(attempt.due(now, true, true)); // explicit regenerate bypasses cache
        attempt.started_at = Some(now);
        assert!(!attempt.due(now + chrono::Duration::seconds(119), false, false));
        assert!(attempt.due(now + chrono::Duration::seconds(120), false, false));
        attempt.failed_at = Some(now);
        attempt.error = Some("Try again".into());
        assert!(!attempt.due(now + chrono::Duration::seconds(599), false, true));
        assert!(attempt.due(now + chrono::Duration::seconds(600), false, true));
        assert!(!attempt.due(now + chrono::Duration::seconds(599), false, false));
        assert!(attempt.due(now + chrono::Duration::seconds(600), false, false));
        assert!(attempt.due(now, true, false));
        attempt.running = true;
        assert!(!attempt.due(now + chrono::Duration::hours(1), true, false));
    }

    #[test]
    fn failures_keep_valid_cache_and_recovery_clears_status() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Laboratory observes unexpected particle behavior",
            "high",
        );
        let current = snapshot(&db, zone, &settings, now).unwrap();
        let old = cache(&current);
        let failed = Attempt {
            error: Some("Try again".into()),
            failed_at: Some(now),
            ..Attempt::default()
        };
        let view = entry(&current, Some(old.clone()), Some(&failed), true);
        assert_eq!(view.status, "error");
        assert!(!view.sentences.is_empty());
        assert_eq!(
            entry(&current, Some(old), Some(&Attempt::default()), true).status,
            "ready"
        );
        assert_eq!(entry(&current, None, None, false).status, "unconfigured");
    }

    #[tokio::test]
    async fn hierarchical_generation_considers_all_articles_and_preserves_original_ids() {
        let (db, feed, zone, settings, now) = fixture();
        add(
            &db,
            feed,
            zone,
            Some(now),
            "Sample evidence about a research experiment",
            "high",
        );
        let mut current = snapshot(&db, zone, &settings, now).unwrap();
        current.articles = (1..=180)
            .map(|id| {
                let mut article = current.articles[0].clone();
                article.id = id;
                article.content = "Evidence detail. ".repeat(30);
                article
            })
            .collect();
        let mut seen = BTreeSet::new();
        let mut calls = 0;
        let result = summarize_with(&current, |payload| {
            calls += 1;
            assert!(payload.chars().count() <= PAYLOAD_LIMIT);
            let items: serde_json::Value = serde_json::from_str(&payload).unwrap();
            let ids: Vec<_> = items.as_array().unwrap().iter()
                .flat_map(|item| item["article_ids"].as_array().unwrap())
                .map(|id| id.as_i64().unwrap()).collect();
            seen.extend(&ids);
            let response = serde_json::json!({"sentences":[{"text":"Researchers reported new findings.","article_ids":[ids[0]]}]}).to_string();
            async move { Ok(response) }
        }).await.unwrap();
        assert!(calls > 2);
        assert_eq!(seen, (1..=180).collect());
        assert!(result[0]
            .article_ids
            .iter()
            .all(|id| current.articles.iter().any(|a| a.id == *id)));
        assert!(
            summarize_with(&current, |_| async { Err("provider unavailable".into()) })
                .await
                .is_err()
        );
    }
}
