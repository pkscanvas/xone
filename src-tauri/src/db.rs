use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;

use crate::dedup::{normalized_title, ArticleIndex, IndexedArticle, INDEX_RETENTION_SECONDS};
use crate::defaults;

const TOPIC_QUERY_RETIREMENT_KEY: &str = "catalog_topic_queries_retired_v1";
const ARTICLE_CLUSTER_RECONCILIATION_KEY: &str = "article_cluster_reconciliation_v2";

const VISIBLE_IN_ANY_ZONE_FILTER: &str = "
    EXISTS (
        SELECT 1
        FROM zone_articles
        INNER JOIN zones ON zones.id = zone_articles.zone_id
        WHERE zone_articles.article_id = articles.id
            AND CASE zone_articles.match_level
                    WHEN 'high' THEN 3
                    WHEN 'medium' THEN 2
                    WHEN 'low' THEN 1
                    ELSE 0
                END >= CASE zones.focus_level
                    WHEN 'laser' THEN 3
                    WHEN 'focused' THEN 2
                    ELSE 1
                END
            AND EXISTS (
                SELECT 1
                FROM article_feeds
                INNER JOIN zone_feeds
                    ON zone_feeds.feed_id = article_feeds.feed_id
                    AND zone_feeds.zone_id = zone_articles.zone_id
                WHERE article_feeds.article_id = articles.id
            )
    )
";

pub struct Db {
    conn: Mutex<Connection>,
    article_index: Mutex<ArticleIndex>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
    pub category: Option<String>,
    pub region: Option<String>,
    pub last_fetched_at: Option<String>,
    pub created_at: String,
    pub article_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Article {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub content: String,
    pub summary: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub is_read: bool,
    pub fetched_at: String,
    pub ai_match_level: Option<String>,
    pub related_articles: Vec<RelatedArticle>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedArticle {
    pub feed_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub url: String,
    pub published_at: Option<String>,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SmartFeedCounts {
    pub today: i64,
    pub all_unread: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Zone {
    pub id: i64,
    pub name: String,
    pub filter_description: String,
    pub focus_level: String,
    pub created_at: String,
    pub unread_count: i64,
    pub last_updated_at: Option<String>,
    pub behavior_profile: String,
    pub behavior_reads_processed: i64,
    pub behavior_reads_pending: i64,
    pub behavior_updated_at: Option<String>,
    pub behavior_last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ArticleForClassification {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BehaviorLearningBatch {
    pub zone_id: i64,
    pub revision: i64,
    pub filter_description: String,
    pub current_profile: String,
    pub event_ids: Vec<i64>,
    pub articles: Vec<ArticleForClassification>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BehaviorReadProgress {
    pub zone_id: i64,
    pub pending_reads: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArticleOpenResult {
    pub newly_read: bool,
    pub unread_zone_ids: Vec<i64>,
    pub behavior_progress: Vec<BehaviorReadProgress>,
    #[serde(skip_serializing)]
    pub ready_behavior_zone_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct FeedMetadata {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
}

/// HTTP conditional-GET validators for one source. Replayed as
/// `If-None-Match` / `If-Modified-Since` so an unchanged feed answers 304.
#[derive(Debug, Clone, Default)]
pub struct FeedCacheValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl FeedCacheValidators {
    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct NewArticle {
    pub title: String,
    pub content: String,
    pub summary: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub published_at: Option<String>,
}

struct ArticleForReconciliation {
    indexed: IndexedArticle,
}

struct StoredClassification {
    zone_id: i64,
    relevant: i64,
    confidence: f64,
    match_level: String,
    classified_at: String,
}

struct StoredReadEvent {
    id: i64,
    zone_id: i64,
    opened_at: String,
    processed_at: Option<String>,
}

impl Db {
    pub fn new(path: &Path) -> Result<Self, String> {
        if path != Path::new(":memory:") {
            crate::security::private_database(path).map_err(|err| err.to_string())?;
        }
        let conn = Connection::open(path).map_err(|err| err.to_string())?;
        let db = Self {
            conn: Mutex::new(conn),
            article_index: Mutex::new(ArticleIndex::default()),
        };
        db.init()?;
        db.seed_default_feeds()?;
        db.reconcile_recent_article_clusters()?;
        db.rebuild_article_index()?;
        Ok(db)
    }

    fn rebuild_article_index(&self) -> Result<(), String> {
        let articles = {
            let conn = self.conn.lock().map_err(|err| err.to_string())?;
            load_existing_index_entries(&conn, &article_index_cutoff())?
        };
        let mut index = self.article_index.lock().map_err(|err| err.to_string())?;
        *index = ArticleIndex::from_articles(articles);
        Ok(())
    }

    fn reconcile_recent_article_clusters(&self) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let already_reconciled = conn
            .query_row(
                "SELECT 1 FROM settings WHERE key = ?1",
                params![ARTICLE_CLUSTER_RECONCILIATION_KEY],
                |_| Ok(()),
            )
            .optional()
            .map_err(|err| err.to_string())?
            .is_some();
        if already_reconciled {
            return Ok(0);
        }

        let cutoff = (Utc::now() - Duration::hours(96)).to_rfc3339();
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let merged = reconcile_article_clusters(&transaction, &cutoff)?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                params![ARTICLE_CLUSTER_RECONCILIATION_KEY, Utc::now().to_rfc3339()],
            )
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(merged)
    }

    #[cfg(test)]
    fn reconcile_article_clusters_since(&self, cutoff: &str) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let merged = reconcile_article_clusters(&transaction, cutoff)?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(merged)
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                description TEXT,
                site_url TEXT,
                category TEXT,
                region TEXT,
                last_fetched_at TEXT,
                etag TEXT,
                last_modified TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                normalized_title TEXT NOT NULL,
                content TEXT NOT NULL,
                summary TEXT,
                url TEXT NOT NULL UNIQUE,
                author TEXT,
                published_at TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                fetched_at TEXT NOT NULL,
                FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS article_feeds (
                article_id INTEGER NOT NULL,
                feed_id INTEGER NOT NULL,
                PRIMARY KEY (article_id, feed_id),
                FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
                FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS related_articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                canonical_article_id INTEGER NOT NULL,
                feed_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                summary TEXT,
                url TEXT NOT NULL UNIQUE,
                published_at TEXT,
                fetched_at TEXT NOT NULL,
                FOREIGN KEY (canonical_article_id) REFERENCES articles(id) ON DELETE CASCADE,
                FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE CASCADE
            );

            INSERT OR IGNORE INTO article_feeds (article_id, feed_id)
            SELECT id, feed_id FROM articles;

            CREATE TABLE IF NOT EXISTS zones (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                filter_description TEXT NOT NULL,
                focus_level TEXT NOT NULL DEFAULT 'focused',
                display_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS zone_summaries (
                zone_id INTEGER PRIMARY KEY,
                cache_json TEXT NOT NULL,
                FOREIGN KEY (zone_id) REFERENCES zones(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS zone_feeds (
                zone_id INTEGER NOT NULL,
                feed_id INTEGER NOT NULL,
                PRIMARY KEY (zone_id, feed_id),
                FOREIGN KEY (zone_id) REFERENCES zones(id) ON DELETE CASCADE,
                FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS zone_articles (
                zone_id INTEGER NOT NULL,
                article_id INTEGER NOT NULL,
                relevant INTEGER NOT NULL,
                confidence REAL NOT NULL,
                match_level TEXT NOT NULL DEFAULT 'no',
                classified_at TEXT NOT NULL,
                PRIMARY KEY (zone_id, article_id),
                FOREIGN KEY (zone_id) REFERENCES zones(id) ON DELETE CASCADE,
                FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS zone_behavior_profiles (
                zone_id INTEGER PRIMARY KEY,
                profile TEXT NOT NULL DEFAULT '',
                processed_reads INTEGER NOT NULL DEFAULT 0,
                revision INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT,
                last_error TEXT,
                FOREIGN KEY (zone_id) REFERENCES zones(id) ON DELETE CASCADE
            );

            INSERT OR IGNORE INTO zone_behavior_profiles (zone_id)
            SELECT id FROM zones;

            CREATE TABLE IF NOT EXISTS zone_read_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                zone_id INTEGER NOT NULL,
                article_id INTEGER NOT NULL,
                opened_at TEXT NOT NULL,
                processed_at TEXT,
                UNIQUE (zone_id, article_id),
                FOREIGN KEY (zone_id) REFERENCES zones(id) ON DELETE CASCADE,
                FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_articles_feed_published
                ON articles(feed_id, published_at DESC);
            CREATE INDEX IF NOT EXISTS idx_articles_normalized_title
                ON articles(normalized_title);
            CREATE INDEX IF NOT EXISTS idx_article_feeds_feed
                ON article_feeds(feed_id);
            CREATE INDEX IF NOT EXISTS idx_related_articles_canonical
                ON related_articles(canonical_article_id);
            CREATE INDEX IF NOT EXISTS idx_zone_feeds_feed
                ON zone_feeds(feed_id);
            -- The primary key is (zone_id, article_id), which cannot serve the
            -- Smart Feed visibility check: that looks an article up across every
            -- zone. Without this, each candidate article scans all of
            -- zone_articles.
            CREATE INDEX IF NOT EXISTS idx_zone_articles_article
                ON zone_articles(article_id, zone_id);
            CREATE INDEX IF NOT EXISTS idx_zone_read_events_pending
                ON zone_read_events(zone_id, processed_at, id);
            ",
        )
        .map_err(|err| err.to_string())?;

        let columns = conn
            .prepare("PRAGMA table_info(feeds)")
            .map_err(|err| err.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        if !columns.iter().any(|column| column == "region") {
            conn.execute("ALTER TABLE feeds ADD COLUMN region TEXT", [])
                .map_err(|err| err.to_string())?;
        }

        // Conditional-GET validators. A feed rarely changes between polls, so
        // replaying these lets the server answer 304 and skips the download,
        // parse, and dedup pass entirely.
        if !columns.iter().any(|column| column == "etag") {
            conn.execute("ALTER TABLE feeds ADD COLUMN etag TEXT", [])
                .map_err(|err| err.to_string())?;
        }
        if !columns.iter().any(|column| column == "last_modified") {
            conn.execute("ALTER TABLE feeds ADD COLUMN last_modified TEXT", [])
                .map_err(|err| err.to_string())?;
        }

        let zone_columns = conn
            .prepare("PRAGMA table_info(zones)")
            .map_err(|err| err.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        if !zone_columns.iter().any(|column| column == "focus_level") {
            conn.execute(
                "ALTER TABLE zones ADD COLUMN focus_level TEXT NOT NULL DEFAULT 'focused'",
                [],
            )
            .map_err(|err| err.to_string())?;
        }
        if !zone_columns.iter().any(|column| column == "display_order") {
            conn.execute(
                "ALTER TABLE zones ADD COLUMN display_order INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|err| err.to_string())?;

            let zone_ids = conn
                .prepare("SELECT id FROM zones ORDER BY name COLLATE NOCASE, id")
                .map_err(|err| err.to_string())?
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|err| err.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| err.to_string())?;
            for (display_order, zone_id) in zone_ids.into_iter().enumerate() {
                conn.execute(
                    "UPDATE zones SET display_order = ?1 WHERE id = ?2",
                    params![display_order as i64, zone_id],
                )
                .map_err(|err| err.to_string())?;
            }
        }

        let classification_columns = conn
            .prepare("PRAGMA table_info(zone_articles)")
            .map_err(|err| err.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        if !classification_columns
            .iter()
            .any(|column| column == "match_level")
        {
            conn.execute(
                "ALTER TABLE zone_articles ADD COLUMN match_level TEXT NOT NULL DEFAULT 'no'",
                [],
            )
            .map_err(|err| err.to_string())?;
            conn.execute(
                "UPDATE zone_articles
                 SET match_level = CASE
                    WHEN relevant = 1 AND confidence > 0 THEN 'medium'
                    ELSE 'no'
                 END",
                [],
            )
            .map_err(|err| err.to_string())?;
        }

        Ok(())
    }

    fn seed_default_feeds(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let now = Utc::now().to_rfc3339();

        for feed in defaults::default_feeds() {
            conn.execute(
                "
                INSERT INTO feeds
                    (url, title, description, site_url, category, region, created_at)
                VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5)
                ON CONFLICT(url) DO UPDATE SET
                    title = excluded.title,
                    category = excluded.category,
                    region = excluded.region
                ",
                params![feed.url, feed.title, feed.category, feed.region, now],
            )
            .map_err(|err| err.to_string())?;
        }

        for &(retired_url, retained_url) in defaults::RETIRED_DEFAULT_FEED_MERGES {
            merge_seeded_feed(&conn, retired_url, retained_url)?;
        }
        // A publisher's own feed supersedes the Google News proxy that used to stand in
        // for it. Merge rather than delete so stored articles, cluster members, and Xone
        // assignments follow the publisher to its native row.
        for &(proxy_url, native_url) in defaults::NATIVE_FEED_REPLACEMENTS {
            // The proxy no longer reaches merge_legacy_google_news_windows, so fold its
            // own legacy windows in here instead of stranding them as orphan rows.
            for legacy_url in legacy_google_news_windows(proxy_url) {
                merge_seeded_feed(&conn, &legacy_url, native_url)?;
            }
            merge_seeded_feed(&conn, proxy_url, native_url)?;
        }
        merge_legacy_google_news_windows(&conn)?;
        retire_legacy_topic_query_feeds(&mut conn)?;

        conn.execute(
            "UPDATE feeds SET category = 'General News' WHERE category IN ('Regional News', 'US News')",
            [],
        )
        .map_err(|err| err.to_string())?;
        conn.execute(
            "UPDATE feeds SET region = NULL WHERE category IS NULL OR category <> 'General News'",
            [],
        )
        .map_err(|err| err.to_string())?;
        conn.execute(
            "UPDATE feeds SET region = 'World' WHERE category = 'General News' AND (region IS NULL OR TRIM(region) = '')",
            [],
        )
        .map_err(|err| err.to_string())?;

        Ok(())
    }

    pub fn seed_default_zones(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let has_zones = transaction
            .query_row("SELECT EXISTS(SELECT 1 FROM zones)", [], |row| {
                row.get::<_, bool>(0)
            })
            .map_err(|err| err.to_string())?;
        if has_zones {
            return Ok(());
        }

        for (order, zone) in defaults::DEFAULT_ZONES.iter().enumerate() {
            transaction.execute(
                "INSERT INTO zones (name, filter_description, focus_level, display_order, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![zone.name, zone.filter, zone.focus, order as i64, Utc::now().to_rfc3339()],
            ).map_err(|err| err.to_string())?;
            let zone_id = transaction.last_insert_rowid();
            transaction.execute(
                "INSERT INTO zone_behavior_profiles (zone_id) VALUES (?1)",
                params![zone_id],
            ).map_err(|err| err.to_string())?;
            for url in zone.feed_urls {
                let feed_id = transaction
                    .query_row("SELECT id FROM feeds WHERE url = ?1", params![url], |row| row.get::<_, i64>(0))
                    .optional().map_err(|err| err.to_string())?
                    .ok_or_else(|| format!("Default Xone source is missing: {url}"))?;
                transaction.execute(
                    "INSERT INTO zone_feeds (zone_id, feed_id) VALUES (?1, ?2)",
                    params![zone_id, feed_id],
                ).map_err(|err| err.to_string())?;
            }
        }
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn list_feeds(&self) -> Result<Vec<Feed>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT id, url, title, description, site_url, category, region,
                    last_fetched_at, created_at,
                    (SELECT COUNT(*) FROM article_feeds WHERE article_feeds.feed_id = feeds.id)
                FROM feeds
                ORDER BY region IS NULL, region COLLATE NOCASE,
                    category COLLATE NOCASE, title COLLATE NOCASE
                ",
            )
            .map_err(|err| err.to_string())?;

        let feeds = stmt
            .query_map([], row_to_feed)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(feeds)
    }

    pub fn get_feed(&self, id: i64) -> Result<Option<Feed>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "
            SELECT id, url, title, description, site_url, category, region,
                last_fetched_at, created_at,
                (SELECT COUNT(*) FROM article_feeds WHERE article_feeds.feed_id = feeds.id)
            FROM feeds
            WHERE id = ?1
            ",
            params![id],
            row_to_feed,
        )
        .optional()
        .map_err(|err| err.to_string())
    }

    /// The stored display name for a URL, if the source already exists.
    /// Callers use this to keep a saved publisher name when re-adding a
    /// source without supplying a new one.
    pub fn feed_title(&self, url: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "SELECT title FROM feeds WHERE url = ?1",
            params![url],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| err.to_string())
    }

    pub fn upsert_feed(
        &self,
        metadata: &FeedMetadata,
        category: Option<&str>,
        region: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "
            INSERT INTO feeds
                (url, title, description, site_url, category, region, created_at, last_fetched_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(url) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                site_url = excluded.site_url,
                category = excluded.category,
                region = excluded.region,
                last_fetched_at = excluded.last_fetched_at
            ",
            params![
                metadata.url,
                metadata.title,
                metadata.description,
                metadata.site_url,
                category,
                region,
                now
            ],
        )
        .map_err(|err| err.to_string())?;

        conn.query_row(
            "SELECT id FROM feeds WHERE url = ?1",
            params![metadata.url],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| err.to_string())
    }

    pub fn update_feed_fetch(&self, id: i64, metadata: &FeedMetadata) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            UPDATE feeds
            SET description = ?1,
                site_url = ?2,
                last_fetched_at = ?3
            WHERE id = ?4
            ",
            params![
                metadata.description,
                metadata.site_url,
                Utc::now().to_rfc3339(),
                id
            ],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// The conditional-GET validators saved for a source by its last
    /// successful fetch. Missing values simply mean the source has never
    /// served one, and the next fetch is unconditional.
    pub fn feed_cache_validators(&self, id: i64) -> Result<FeedCacheValidators, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "SELECT etag, last_modified FROM feeds WHERE id = ?1",
            params![id],
            |row| {
                Ok(FeedCacheValidators {
                    etag: row.get(0)?,
                    last_modified: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| err.to_string())
        .map(Option::unwrap_or_default)
    }

    pub fn save_feed_cache_validators(
        &self,
        id: i64,
        validators: &FeedCacheValidators,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "UPDATE feeds SET etag = ?1, last_modified = ?2 WHERE id = ?3",
            params![validators.etag, validators.last_modified, id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Record a successful poll that returned no new content. Metadata is left
    /// alone because a 304 carries none.
    pub fn touch_feed_fetch(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "UPDATE feeds SET last_fetched_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn remove_feed(&self, id: i64) -> Result<(), String> {
        let removed = {
            let conn = self.conn.lock().map_err(|err| err.to_string())?;
            remove_feed_preserving_shared_articles(&conn, id)?
        };
        if removed == 0 {
            return Err("Source not found".to_string());
        }
        self.rebuild_article_index()?;
        Ok(())
    }

    pub fn count_articles_older_than(&self, cutoff: &str) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "
            SELECT COUNT(*)
            FROM articles
            WHERE datetime(COALESCE(published_at, fetched_at)) < datetime(?1)
            ",
            params![cutoff],
            |row| row.get(0),
        )
        .map_err(|err| err.to_string())
    }

    pub fn delete_articles_older_than(&self, cutoff: &str) -> Result<usize, String> {
        let deleted = {
            let conn = self.conn.lock().map_err(|err| err.to_string())?;
            conn.execute(
                "
                DELETE FROM articles
                WHERE datetime(COALESCE(published_at, fetched_at)) < datetime(?1)
                ",
                params![cutoff],
            )
            .map_err(|err| err.to_string())?
        };
        if deleted > 0 {
            self.rebuild_article_index()?;
        }
        Ok(deleted)
    }

    pub fn compact(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute_batch("VACUUM").map_err(|err| err.to_string())
    }

    pub fn insert_articles(&self, feed_id: i64, articles: &[NewArticle]) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut article_index = self.article_index.lock().map_err(|err| err.to_string())?;
        article_index.prune_before(article_index_cutoff_timestamp());
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        // Articles are indexed as they are inserted so later entries in the
        // same batch are matched against them with the full corpus token
        // statistics; the checkpoint undoes those inserts if the transaction
        // never lands.
        let checkpoint = article_index.checkpoint();
        let inserted = match insert_batch(&transaction, &mut article_index, feed_id, articles) {
            Ok(inserted) => inserted,
            Err(err) => {
                article_index.rollback_to(checkpoint);
                return Err(err);
            }
        };

        if let Err(err) = transaction.commit() {
            article_index.rollback_to(checkpoint);
            return Err(err.to_string());
        }
        Ok(inserted)
    }
}

/// Inserts one feed's articles, clustering each against everything already
/// indexed — including earlier articles from this same batch.
fn insert_batch(
    transaction: &Transaction<'_>,
    article_index: &mut ArticleIndex,
    feed_id: i64,
    articles: &[NewArticle],
) -> Result<usize, String> {
    let mut inserted = 0;

    for article in articles {
        let existing_url = transaction
            .query_row(
                "
                SELECT id FROM articles WHERE url = ?1
                UNION ALL
                SELECT canonical_article_id FROM related_articles WHERE url = ?1
                LIMIT 1
                ",
                params![article.url],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;

        if let Some(article_id) = existing_url {
            if associate_article(transaction, article_id, feed_id)? {
                inserted += 1;
            }
            continue;
        }

        let normalized = normalized_title(&article.title);
        let fetched_at = Utc::now().to_rfc3339();
        let event_at = article
            .published_at
            .clone()
            .unwrap_or_else(|| fetched_at.clone());
        let duplicate_id =
            article_index.find_duplicate(&article.title, article.summary.as_deref(), &event_at);
        if let Some(article_id) = duplicate_id {
            insert_related_article(transaction, article_id, feed_id, article)?;
            if associate_article(transaction, article_id, feed_id)? {
                inserted += 1;
            }
            continue;
        }

        transaction
            .execute(
                "
                INSERT INTO articles
                    (feed_id, title, normalized_title, content, summary, url, author, published_at, fetched_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
                params![
                    feed_id,
                    article.title,
                    normalized,
                    article.content,
                    article.summary,
                    article.url,
                    article.author,
                    article.published_at,
                    fetched_at
                ],
            )
            .map_err(|err| err.to_string())?;

        let article_id = transaction.last_insert_rowid();
        if associate_article(transaction, article_id, feed_id)? {
            inserted += 1;
        }
        article_index.insert(
            article_id,
            &article.title,
            article.summary.as_deref(),
            &event_at,
        );
    }

    Ok(inserted)
}

impl Db {
    pub fn list_articles(&self, feed_id: i64) -> Result<Vec<Article>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT articles.id, article_feeds.feed_id, articles.title, articles.content,
                    articles.summary, articles.url, articles.author,
                    articles.published_at, articles.is_read, articles.fetched_at, NULL
                FROM articles
                INNER JOIN article_feeds ON article_feeds.article_id = articles.id
                WHERE article_feeds.feed_id = ?1
                ORDER BY COALESCE(articles.published_at, articles.fetched_at) DESC
                ",
            )
            .map_err(|err| err.to_string())?;

        let mut articles = stmt
            .query_map(params![feed_id], row_to_article)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        load_related_articles(&conn, &mut articles)?;
        Ok(articles)
    }

    pub fn smart_feed_counts(&self) -> Result<SmartFeedCounts, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let sql = format!(
            "
            SELECT
                COUNT(CASE
                    WHEN datetime(COALESCE(published_at, fetched_at)) >=
                            datetime('now', 'localtime', 'start of day', 'utc')
                        AND datetime(COALESCE(published_at, fetched_at)) <
                            datetime('now', 'localtime', 'start of day', '+1 day', 'utc')
                    THEN 1
                END),
                COUNT(CASE WHEN is_read = 0 THEN 1 END)
            FROM articles
            WHERE {VISIBLE_IN_ANY_ZONE_FILTER}
            "
        );
        conn.query_row(&sql, [], |row| {
            Ok(SmartFeedCounts {
                today: row.get(0)?,
                all_unread: row.get(1)?,
            })
        })
        .map_err(|err| err.to_string())
    }

    pub fn list_today_articles(&self) -> Result<Vec<Article>, String> {
        self.list_smart_feed_articles(
            "datetime(COALESCE(articles.published_at, articles.fetched_at)) >= \
                datetime('now', 'localtime', 'start of day', 'utc') \
            AND datetime(COALESCE(articles.published_at, articles.fetched_at)) < \
                datetime('now', 'localtime', 'start of day', '+1 day', 'utc')",
        )
    }

    pub fn list_unread_articles(&self) -> Result<Vec<Article>, String> {
        self.list_smart_feed_articles("articles.is_read = 0")
    }

    fn list_smart_feed_articles(&self, filter: &str) -> Result<Vec<Article>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "
                SELECT articles.id, articles.feed_id, articles.title, articles.content,
                    articles.summary, articles.url, articles.author, articles.published_at,
                    articles.is_read, articles.fetched_at, NULL
                FROM articles
                WHERE {VISIBLE_IN_ANY_ZONE_FILTER}
                    AND ({filter})
                ORDER BY COALESCE(articles.published_at, articles.fetched_at) DESC
                "
            ))
            .map_err(|err| err.to_string())?;

        let mut articles = stmt
            .query_map([], row_to_article)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        load_related_articles(&conn, &mut articles)?;
        Ok(articles)
    }

    pub fn list_zones(&self) -> Result<Vec<Zone>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT zones.id, zones.name, zones.filter_description, zones.focus_level,
                    zones.created_at,
                    (
                        SELECT COUNT(*)
                        FROM articles
                        INNER JOIN zone_articles
                            ON zone_articles.article_id = articles.id
                            AND zone_articles.zone_id = zones.id
                        WHERE EXISTS (
                                SELECT 1
                                FROM article_feeds
                                INNER JOIN zone_feeds
                                    ON zone_feeds.feed_id = article_feeds.feed_id
                                WHERE article_feeds.article_id = articles.id
                                    AND zone_feeds.zone_id = zones.id
                            )
                            AND CASE zone_articles.match_level
                                    WHEN 'high' THEN 3
                                    WHEN 'medium' THEN 2
                                    WHEN 'low' THEN 1
                                    ELSE 0
                                END >= CASE zones.focus_level
                                    WHEN 'laser' THEN 3
                                    WHEN 'focused' THEN 2
                                    ELSE 1
                                END
                            AND articles.is_read = 0
                    ) AS unread_count,
                    (
                        SELECT MAX(feeds.last_fetched_at)
                        FROM zone_feeds AS updated_zone_feeds
                        INNER JOIN feeds ON feeds.id = updated_zone_feeds.feed_id
                        WHERE updated_zone_feeds.zone_id = zones.id
                    ) AS last_updated_at,
                    COALESCE(zone_behavior_profiles.profile, '') AS behavior_profile,
                    COALESCE(zone_behavior_profiles.processed_reads, 0) AS behavior_reads_processed,
                    (
                        SELECT COUNT(*)
                        FROM zone_read_events
                        WHERE zone_read_events.zone_id = zones.id
                            AND zone_read_events.processed_at IS NULL
                    ) AS behavior_reads_pending,
                    zone_behavior_profiles.updated_at AS behavior_updated_at,
                    zone_behavior_profiles.last_error AS behavior_last_error
                FROM zones
                LEFT JOIN zone_behavior_profiles
                    ON zone_behavior_profiles.zone_id = zones.id
                ORDER BY zones.display_order, zones.id
                ",
            )
            .map_err(|err| err.to_string())?;

        let zones = stmt
            .query_map([], row_to_zone)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(zones)
    }

    pub fn get_zone(&self, id: i64) -> Result<Option<Zone>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "
            SELECT zones.id, zones.name, zones.filter_description, zones.focus_level,
                zones.created_at,
                (
                    SELECT COUNT(*)
                    FROM articles
                    INNER JOIN zone_articles
                        ON zone_articles.article_id = articles.id
                        AND zone_articles.zone_id = zones.id
                    WHERE EXISTS (
                            SELECT 1
                            FROM article_feeds
                            INNER JOIN zone_feeds
                                ON zone_feeds.feed_id = article_feeds.feed_id
                            WHERE article_feeds.article_id = articles.id
                                AND zone_feeds.zone_id = zones.id
                        )
                        AND CASE zone_articles.match_level
                                WHEN 'high' THEN 3
                                WHEN 'medium' THEN 2
                                WHEN 'low' THEN 1
                                ELSE 0
                            END >= CASE zones.focus_level
                                WHEN 'laser' THEN 3
                                WHEN 'focused' THEN 2
                                ELSE 1
                            END
                        AND articles.is_read = 0
                ) AS unread_count,
                (
                    SELECT MAX(feeds.last_fetched_at)
                    FROM zone_feeds AS updated_zone_feeds
                    INNER JOIN feeds ON feeds.id = updated_zone_feeds.feed_id
                    WHERE updated_zone_feeds.zone_id = zones.id
                ) AS last_updated_at,
                COALESCE(zone_behavior_profiles.profile, '') AS behavior_profile,
                COALESCE(zone_behavior_profiles.processed_reads, 0) AS behavior_reads_processed,
                (
                    SELECT COUNT(*)
                    FROM zone_read_events
                    WHERE zone_read_events.zone_id = zones.id
                        AND zone_read_events.processed_at IS NULL
                ) AS behavior_reads_pending,
                zone_behavior_profiles.updated_at AS behavior_updated_at,
                zone_behavior_profiles.last_error AS behavior_last_error
            FROM zones
            LEFT JOIN zone_behavior_profiles
                ON zone_behavior_profiles.zone_id = zones.id
            WHERE zones.id = ?1
            ",
            params![id],
            row_to_zone,
        )
        .optional()
        .map_err(|err| err.to_string())
    }

    pub fn create_zone(
        &self,
        name: &str,
        filter_description: &str,
        focus_level: &str,
    ) -> Result<i64, String> {
        validate_focus_level(focus_level)?;
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        transaction
            .execute(
                "
            INSERT INTO zones (name, filter_description, focus_level, display_order, created_at)
            VALUES (
                ?1,
                ?2,
                ?3,
                COALESCE((SELECT MAX(display_order) + 1 FROM zones), 0),
                ?4
            )
            ",
                params![
                    name,
                    filter_description,
                    focus_level,
                    Utc::now().to_rfc3339()
                ],
            )
            .map_err(|err| err.to_string())?;
        let zone_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "INSERT INTO zone_behavior_profiles (zone_id) VALUES (?1)",
                params![zone_id],
            )
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(zone_id)
    }

    pub fn update_zone(
        &self,
        id: i64,
        name: &str,
        filter_description: &str,
        focus_level: &str,
    ) -> Result<(), String> {
        validate_focus_level(focus_level)?;
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let previous_filter = transaction
            .query_row(
                "SELECT filter_description FROM zones WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "Xone not found".to_string())?;
        transaction
            .execute(
                "
            UPDATE zones
            SET name = ?1,
                filter_description = ?2,
                focus_level = ?3
            WHERE id = ?4
            ",
                params![name, filter_description, focus_level, id],
            )
            .map_err(|err| err.to_string())?;
        if previous_filter.trim() != filter_description.trim() {
            reset_zone_behavior_in_transaction(&transaction, id)?;
        }
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn delete_zone(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let removed = conn
            .execute("DELETE FROM zones WHERE id = ?1", params![id])
            .map_err(|err| err.to_string())?;
        if removed == 0 {
            return Err("Xone not found".to_string());
        }
        Ok(())
    }

    pub fn reorder_zones(&self, ordered_ids: &[i64]) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut existing_ids = conn
            .prepare("SELECT id FROM zones ORDER BY id")
            .map_err(|err| err.to_string())?
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        let mut requested_ids = ordered_ids.to_vec();
        requested_ids.sort_unstable();
        requested_ids.dedup();
        existing_ids.sort_unstable();
        if requested_ids != existing_ids || ordered_ids.len() != existing_ids.len() {
            return Err("Xone order is out of date. Reload and try again.".to_string());
        }

        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        for (display_order, zone_id) in ordered_ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE zones SET display_order = ?1 WHERE id = ?2",
                    params![display_order as i64, zone_id],
                )
                .map_err(|err| err.to_string())?;
        }
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn assign_feed(&self, zone_id: i64, feed_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            INSERT OR IGNORE INTO zone_feeds (zone_id, feed_id)
            VALUES (?1, ?2)
            ",
            params![zone_id, feed_id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn unassign_feed(&self, zone_id: i64, feed_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            DELETE FROM zone_feeds
            WHERE zone_id = ?1 AND feed_id = ?2
            ",
            params![zone_id, feed_id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn get_zone_feeds(&self, zone_id: i64) -> Result<Vec<Feed>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT feeds.id, feeds.url, feeds.title, feeds.description, feeds.site_url,
                    feeds.category, feeds.region, feeds.last_fetched_at, feeds.created_at,
                    (SELECT COUNT(*) FROM article_feeds WHERE article_feeds.feed_id = feeds.id)
                FROM feeds
                INNER JOIN zone_feeds ON zone_feeds.feed_id = feeds.id
                WHERE zone_feeds.zone_id = ?1
                ORDER BY feeds.region IS NULL, feeds.region COLLATE NOCASE,
                    feeds.category COLLATE NOCASE, feeds.title COLLATE NOCASE
                ",
            )
            .map_err(|err| err.to_string())?;

        let feeds = stmt
            .query_map(params![zone_id], row_to_feed)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(feeds)
    }

    pub fn get_zone_summary(&self, zone_id: i64) -> Result<Option<String>, String> {
        self.conn
            .lock()
            .map_err(|err| err.to_string())?
            .query_row(
                "SELECT cache_json FROM zone_summaries WHERE zone_id = ?1",
                [zone_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| err.to_string())
    }

    pub fn save_zone_summary(&self, zone_id: i64, cache: &str) -> Result<(), String> {
        self.conn
            .lock()
            .map_err(|err| err.to_string())?
            .execute(
                "INSERT INTO zone_summaries (zone_id, cache_json) VALUES (?1, ?2)
                ON CONFLICT(zone_id) DO UPDATE SET cache_json = excluded.cache_json",
                params![zone_id, cache],
            )
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn list_zone_articles(&self, zone_id: i64) -> Result<Vec<Article>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT articles.id,
                    COALESCE((
                        SELECT article_feeds.feed_id
                        FROM article_feeds
                        INNER JOIN zone_feeds
                            ON zone_feeds.feed_id = article_feeds.feed_id
                            AND zone_feeds.zone_id = ?1
                        WHERE article_feeds.article_id = articles.id
                        ORDER BY article_feeds.feed_id
                        LIMIT 1
                    ), articles.feed_id) AS feed_id,
                    articles.title, articles.content,
                    articles.summary, articles.url, articles.author, articles.published_at,
                    articles.is_read, articles.fetched_at, zone_articles.match_level
                FROM articles
                INNER JOIN zone_articles
                    ON zone_articles.article_id = articles.id
                    AND zone_articles.zone_id = ?1
                INNER JOIN zones ON zones.id = zone_articles.zone_id
                WHERE EXISTS (
                        SELECT 1
                        FROM article_feeds
                        INNER JOIN zone_feeds ON zone_feeds.feed_id = article_feeds.feed_id
                        WHERE article_feeds.article_id = articles.id
                            AND zone_feeds.zone_id = ?1
                    )
                    AND CASE zone_articles.match_level
                            WHEN 'high' THEN 3
                            WHEN 'medium' THEN 2
                            WHEN 'low' THEN 1
                            ELSE 0
                        END >= CASE zones.focus_level
                            WHEN 'laser' THEN 3
                            WHEN 'focused' THEN 2
                            ELSE 1
                        END
                ORDER BY COALESCE(articles.published_at, articles.fetched_at) DESC
                ",
            )
            .map_err(|err| err.to_string())?;

        let mut articles = stmt
            .query_map(params![zone_id], row_to_article)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        load_related_articles(&conn, &mut articles)?;
        Ok(articles)
    }

    pub fn list_zone_articles_for_classification(
        &self,
        zone_id: i64,
        reclassify: bool,
        cutoff: Option<&str>,
    ) -> Result<Vec<ArticleForClassification>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut sql = "
            SELECT articles.id, articles.title, articles.content, articles.summary
            FROM articles
            LEFT JOIN zone_articles
                ON zone_articles.article_id = articles.id
                AND zone_articles.zone_id = ?1
            WHERE EXISTS (
                SELECT 1
                FROM article_feeds
                INNER JOIN zone_feeds ON zone_feeds.feed_id = article_feeds.feed_id
                WHERE article_feeds.article_id = articles.id
                    AND zone_feeds.zone_id = ?1
            )
        "
        .to_string();

        if !reclassify {
            sql.push_str(" AND zone_articles.article_id IS NULL");
        }

        sql.push_str(
            " AND (?2 IS NULL OR datetime(COALESCE(articles.published_at, articles.fetched_at)) >= datetime(?2))",
        );

        sql.push_str(" ORDER BY COALESCE(articles.published_at, articles.fetched_at) DESC");

        let mut stmt = conn.prepare(&sql).map_err(|err| err.to_string())?;
        let articles = stmt
            .query_map(params![zone_id, cutoff], |row| {
                Ok(ArticleForClassification {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    summary: row.get(3)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(articles)
    }

    pub fn save_classification(
        &self,
        zone_id: i64,
        article_id: i64,
        match_level: &str,
    ) -> Result<(), String> {
        let (relevant, confidence) = match match_level {
            "no" => (0, 0.0),
            "low" => (1, 0.33),
            "medium" => (1, 0.67),
            "high" => (1, 1.0),
            _ => return Err("Invalid article match level".to_string()),
        };
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            INSERT INTO zone_articles
                (zone_id, article_id, relevant, confidence, match_level, classified_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(zone_id, article_id) DO UPDATE SET
                relevant = excluded.relevant,
                confidence = excluded.confidence,
                match_level = excluded.match_level,
                classified_at = excluded.classified_at
            ",
            params![
                zone_id,
                article_id,
                relevant,
                confidence,
                match_level,
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn list_zones_for_feed(&self, feed_id: i64) -> Result<Vec<Zone>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT zones.id, zones.name, zones.filter_description, zones.focus_level,
                    zones.created_at,
                    (
                        SELECT COUNT(*)
                        FROM articles
                        INNER JOIN zone_articles
                            ON zone_articles.article_id = articles.id
                            AND zone_articles.zone_id = zones.id
                        WHERE EXISTS (
                                SELECT 1
                                FROM article_feeds
                                INNER JOIN zone_feeds AS count_zone_feeds
                                    ON count_zone_feeds.feed_id = article_feeds.feed_id
                                WHERE article_feeds.article_id = articles.id
                                    AND count_zone_feeds.zone_id = zones.id
                            )
                            AND CASE zone_articles.match_level
                                    WHEN 'high' THEN 3
                                    WHEN 'medium' THEN 2
                                    WHEN 'low' THEN 1
                                    ELSE 0
                                END >= CASE zones.focus_level
                                    WHEN 'laser' THEN 3
                                    WHEN 'focused' THEN 2
                                    ELSE 1
                                END
                            AND articles.is_read = 0
                    ) AS unread_count,
                    (
                        SELECT MAX(feeds.last_fetched_at)
                        FROM zone_feeds AS updated_zone_feeds
                        INNER JOIN feeds ON feeds.id = updated_zone_feeds.feed_id
                        WHERE updated_zone_feeds.zone_id = zones.id
                    ) AS last_updated_at,
                    COALESCE(zone_behavior_profiles.profile, '') AS behavior_profile,
                    COALESCE(zone_behavior_profiles.processed_reads, 0) AS behavior_reads_processed,
                    (
                        SELECT COUNT(*)
                        FROM zone_read_events
                        WHERE zone_read_events.zone_id = zones.id
                            AND zone_read_events.processed_at IS NULL
                    ) AS behavior_reads_pending,
                    zone_behavior_profiles.updated_at AS behavior_updated_at,
                    zone_behavior_profiles.last_error AS behavior_last_error
                FROM zones
                INNER JOIN zone_feeds ON zone_feeds.zone_id = zones.id
                LEFT JOIN zone_behavior_profiles
                    ON zone_behavior_profiles.zone_id = zones.id
                WHERE zone_feeds.feed_id = ?1
                ORDER BY zones.display_order, zones.id
                ",
            )
            .map_err(|err| err.to_string())?;

        let zones = stmt
            .query_map(params![feed_id], row_to_zone)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(zones)
    }

    pub fn record_article_open(
        &self,
        article_id: i64,
        direct_zone_id: Option<i64>,
        smart_feed: bool,
        behavior_batch_size: usize,
    ) -> Result<ArticleOpenResult, String> {
        if direct_zone_id.is_some() && smart_feed {
            return Err("Invalid article-open context".to_string());
        }

        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let was_read = transaction
            .query_row(
                "SELECT is_read FROM articles WHERE id = ?1",
                params![article_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;
        let was_read = was_read.ok_or_else(|| "Article not found".to_string())?;
        let newly_read = !was_read;
        if newly_read {
            transaction
                .execute(
                    "UPDATE articles SET is_read = 1 WHERE id = ?1",
                    params![article_id],
                )
                .map_err(|err| err.to_string())?;
        }

        let visible_zone_ids = if newly_read || direct_zone_id.is_some() || smart_feed {
            visible_zone_ids_for_article(&transaction, article_id, None)?
        } else {
            Vec::new()
        };
        let attributed_zone_ids = if let Some(zone_id) = direct_zone_id {
            visible_zone_ids
                .iter()
                .copied()
                .filter(|visible_zone_id| *visible_zone_id == zone_id)
                .collect::<Vec<_>>()
        } else if smart_feed {
            visible_zone_ids.clone()
        } else {
            Vec::new()
        };
        let opened_at = Utc::now().to_rfc3339();
        let mut newly_attributed_zone_ids = Vec::new();
        for zone_id in &attributed_zone_ids {
            let inserted = transaction
                .execute(
                    "
                    INSERT OR IGNORE INTO zone_read_events (zone_id, article_id, opened_at)
                    VALUES (?1, ?2, ?3)
                    ",
                    params![zone_id, article_id, opened_at],
                )
                .map_err(|err| err.to_string())?;
            if inserted > 0 {
                newly_attributed_zone_ids.push(*zone_id);
            }
        }

        let mut behavior_progress = Vec::new();
        let mut ready_behavior_zone_ids = Vec::new();
        for zone_id in newly_attributed_zone_ids {
            let pending_reads = transaction
                .query_row(
                    "
                    SELECT COUNT(*)
                    FROM zone_read_events
                    WHERE zone_id = ?1 AND processed_at IS NULL
                    ",
                    params![zone_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| err.to_string())?;
            behavior_progress.push(BehaviorReadProgress {
                zone_id,
                pending_reads,
            });
            if pending_reads >= behavior_batch_size as i64 {
                ready_behavior_zone_ids.push(zone_id);
            }
        }

        transaction.commit().map_err(|err| err.to_string())?;
        Ok(ArticleOpenResult {
            newly_read,
            unread_zone_ids: if newly_read {
                visible_zone_ids
            } else {
                Vec::new()
            },
            behavior_progress,
            ready_behavior_zone_ids,
        })
    }

    pub fn ready_behavior_zone_ids(&self, batch_size: usize) -> Result<Vec<i64>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT zone_id
                FROM zone_read_events
                WHERE processed_at IS NULL
                GROUP BY zone_id
                HAVING COUNT(*) >= ?1
                ORDER BY zone_id
                ",
            )
            .map_err(|err| err.to_string())?;
        let zone_ids = stmt
            .query_map(params![batch_size as i64], |row| row.get(0))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        Ok(zone_ids)
    }

    pub fn next_behavior_batch(
        &self,
        zone_id: i64,
        batch_size: usize,
    ) -> Result<Option<BehaviorLearningBatch>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let state = conn
            .query_row(
                "
                SELECT zones.filter_description,
                    zone_behavior_profiles.profile,
                    zone_behavior_profiles.revision
                FROM zones
                INNER JOIN zone_behavior_profiles
                    ON zone_behavior_profiles.zone_id = zones.id
                WHERE zones.id = ?1
                ",
                params![zone_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|err| err.to_string())?;
        let Some((filter_description, current_profile, revision)) = state else {
            return Ok(None);
        };

        let mut stmt = conn
            .prepare(
                "
                SELECT zone_read_events.id, articles.id, articles.title,
                    articles.content, articles.summary
                FROM zone_read_events
                INNER JOIN articles ON articles.id = zone_read_events.article_id
                WHERE zone_read_events.zone_id = ?1
                    AND zone_read_events.processed_at IS NULL
                ORDER BY zone_read_events.id
                LIMIT ?2
                ",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(params![zone_id, batch_size as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    ArticleForClassification {
                        id: row.get(1)?,
                        title: row.get(2)?,
                        content: row.get(3)?,
                        summary: row.get(4)?,
                    },
                ))
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        if rows.len() < batch_size {
            return Ok(None);
        }
        let (event_ids, articles) = rows.into_iter().unzip();
        Ok(Some(BehaviorLearningBatch {
            zone_id,
            revision,
            filter_description,
            current_profile,
            event_ids,
            articles,
        }))
    }

    pub fn complete_behavior_batch(
        &self,
        batch: &BehaviorLearningBatch,
        profile: &str,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        let current_revision = transaction
            .query_row(
                "SELECT revision FROM zone_behavior_profiles WHERE zone_id = ?1",
                params![batch.zone_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;
        if current_revision != Some(batch.revision) {
            return Ok(false);
        }

        let processed_at = Utc::now().to_rfc3339();
        for event_id in &batch.event_ids {
            transaction
                .execute(
                    "
                    UPDATE zone_read_events
                    SET processed_at = ?1
                    WHERE id = ?2 AND zone_id = ?3 AND processed_at IS NULL
                    ",
                    params![processed_at, event_id, batch.zone_id],
                )
                .map_err(|err| err.to_string())?;
        }
        transaction
            .execute(
                "
                UPDATE zone_behavior_profiles
                SET profile = ?1,
                    processed_reads = processed_reads + ?2,
                    updated_at = ?3,
                    last_error = NULL
                WHERE zone_id = ?4 AND revision = ?5
                ",
                params![
                    profile,
                    batch.event_ids.len() as i64,
                    processed_at,
                    batch.zone_id,
                    batch.revision
                ],
            )
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(true)
    }

    pub fn fail_behavior_batch(
        &self,
        zone_id: i64,
        revision: i64,
        error: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            UPDATE zone_behavior_profiles
            SET last_error = ?1
            WHERE zone_id = ?2 AND revision = ?3
            ",
            params![error, zone_id, revision],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn reset_zone_behavior(&self, zone_id: i64) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let transaction = conn.transaction().map_err(|err| err.to_string())?;
        reset_zone_behavior_in_transaction(&transaction, zone_id)?;
        transaction.commit().map_err(|err| err.to_string())
    }

    pub fn get_settings(&self) -> Result<Vec<Setting>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let mut stmt = conn
            .prepare(
                "
                SELECT key, value
                FROM settings
                ORDER BY key COLLATE NOCASE
                ",
            )
            .map_err(|err| err.to_string())?;

        let settings = stmt
            .query_map([], row_to_setting)
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        Ok(settings)
    }

    pub fn update_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "
            INSERT INTO settings (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
            params![key, value],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn update_settings(&self, settings: &[Setting]) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|err| err.to_string())?;
        let tx = conn.transaction().map_err(|err| err.to_string())?;
        {
            let mut statement = tx
                .prepare(
                    "
                    INSERT INTO settings (key, value)
                    VALUES (?1, ?2)
                    ON CONFLICT(key) DO UPDATE SET value = excluded.value
                    ",
                )
                .map_err(|err| err.to_string())?;
            for setting in settings {
                statement
                    .execute(params![&setting.key, &setting.value])
                    .map_err(|err| err.to_string())?;
            }
        }
        tx.commit().map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "
            SELECT value
            FROM settings
            WHERE key = ?1
            ",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| err.to_string())
    }
}

fn retire_legacy_topic_query_feeds(conn: &mut Connection) -> Result<(), String> {
    let already_applied = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key = ?1)",
            params![TOPIC_QUERY_RETIREMENT_KEY],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|err| err.to_string())?;
    if already_applied {
        return Ok(());
    }

    let transaction = conn.transaction().map_err(|err| err.to_string())?;
    for feed in defaults::retired_topic_query_feeds() {
        retire_seeded_feed(&transaction, feed.url)?;
    }
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, '1')",
            params![TOPIC_QUERY_RETIREMENT_KEY],
        )
        .map_err(|err| err.to_string())?;
    transaction.commit().map_err(|err| err.to_string())?;
    Ok(())
}

/// The pre-cap URLs a four-day publisher-scoped Google News source may still be stored
/// under. Returns nothing for URLs that are not four-day Google News searches.
fn legacy_google_news_windows(url: &str) -> Vec<String> {
    if !url.starts_with("https://news.google.com/rss/search?") || !url.contains("when:4d") {
        return Vec::new();
    }
    let mut legacy = ["when:7d", "when:14d", "when:30d"]
        .iter()
        .map(|window| url.replace("when:4d", window))
        .collect::<Vec<_>>();
    legacy.push(url.replace("+when:4d", ""));
    legacy
}

fn merge_legacy_google_news_windows(conn: &Connection) -> Result<(), String> {
    for feed in defaults::default_feeds() {
        for legacy_url in legacy_google_news_windows(feed.url) {
            merge_seeded_feed(conn, &legacy_url, feed.url)?;
        }
    }
    Ok(())
}

fn retire_seeded_feed(conn: &Connection, retired_url: &str) -> Result<(), String> {
    let retired_id = conn
        .query_row(
            "SELECT id FROM feeds WHERE url = ?1",
            params![retired_url],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let Some(retired_id) = retired_id else {
        return Ok(());
    };

    remove_feed_preserving_shared_articles(conn, retired_id)?;
    Ok(())
}

fn remove_feed_preserving_shared_articles(
    conn: &Connection,
    feed_id: i64,
) -> Result<usize, String> {
    conn.execute(
        "
        UPDATE articles
        SET feed_id = (
            SELECT article_feeds.feed_id
            FROM article_feeds
            WHERE article_feeds.article_id = articles.id
                AND article_feeds.feed_id <> ?1
            LIMIT 1
        )
        WHERE articles.feed_id = ?1
            AND EXISTS (
                SELECT 1
                FROM article_feeds
                WHERE article_feeds.article_id = articles.id
                    AND article_feeds.feed_id <> ?1
        )
        ",
        params![feed_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute("DELETE FROM feeds WHERE id = ?1", params![feed_id])
        .map_err(|err| err.to_string())
}

fn merge_seeded_feed(
    conn: &Connection,
    retired_url: &str,
    retained_url: &str,
) -> Result<(), String> {
    let retired_id = conn
        .query_row(
            "SELECT id FROM feeds WHERE url = ?1",
            params![retired_url],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let retained_id = conn
        .query_row(
            "SELECT id FROM feeds WHERE url = ?1",
            params![retained_url],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?;

    let (Some(retired_id), Some(retained_id)) = (retired_id, retained_id) else {
        return Ok(());
    };
    if retired_id == retained_id {
        return Ok(());
    }

    conn.execute(
        "
        INSERT OR IGNORE INTO zone_feeds (zone_id, feed_id)
        SELECT zone_id, ?1 FROM zone_feeds WHERE feed_id = ?2
        ",
        params![retained_id, retired_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute(
        "UPDATE articles SET feed_id = ?1 WHERE feed_id = ?2",
        params![retained_id, retired_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute(
        "UPDATE related_articles SET feed_id = ?1 WHERE feed_id = ?2",
        params![retained_id, retired_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute(
        "
        INSERT OR IGNORE INTO article_feeds (article_id, feed_id)
        SELECT article_id, ?1 FROM article_feeds WHERE feed_id = ?2
        ",
        params![retained_id, retired_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute("DELETE FROM feeds WHERE id = ?1", params![retired_id])
        .map_err(|err| err.to_string())?;

    Ok(())
}

fn associate_article(conn: &Connection, article_id: i64, feed_id: i64) -> Result<bool, String> {
    conn.execute(
        "INSERT OR IGNORE INTO article_feeds (article_id, feed_id) VALUES (?1, ?2)",
        params![article_id, feed_id],
    )
    .map(|inserted| inserted > 0)
    .map_err(|err| err.to_string())
}

fn reconcile_article_clusters(
    transaction: &Transaction<'_>,
    cutoff: &str,
) -> Result<usize, String> {
    let candidates = {
        let mut stmt = transaction
            .prepare(
                "
                SELECT id, title, summary, COALESCE(published_at, fetched_at)
                FROM articles
                WHERE datetime(COALESCE(published_at, fetched_at)) >= datetime(?1)
                ORDER BY is_read DESC,
                    CASE
                        WHEN summary IS NULL
                            OR summary LIKE '%Comments URL:%'
                            OR summary LIKE 'Article URL:%'
                            OR summary LIKE 'http%'
                        THEN 0
                        ELSE length(summary)
                    END DESC,
                    datetime(COALESCE(published_at, fetched_at)), id
                ",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(params![cutoff], |row| {
                Ok(ArticleForReconciliation {
                    indexed: IndexedArticle {
                        article_id: row.get(0)?,
                        title: row.get(1)?,
                        summary: row.get(2)?,
                        event_at: row.get(3)?,
                    },
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        rows
    };

    let mut index = ArticleIndex::default();
    let mut merged = 0;
    for candidate in candidates {
        let article = candidate.indexed;
        let duplicate_id = index.find_duplicate(
            &article.title,
            article.summary.as_deref(),
            &article.event_at,
        );

        let canonical_id = if let Some(canonical_id) = duplicate_id {
            merge_canonical_articles(transaction, canonical_id, article.article_id)?;
            merged += 1;
            canonical_id
        } else {
            article.article_id
        };

        index.insert(
            canonical_id,
            &article.title,
            article.summary.as_deref(),
            &article.event_at,
        );
    }

    Ok(merged)
}

fn merge_canonical_articles(
    transaction: &Transaction<'_>,
    canonical_article_id: i64,
    duplicate_article_id: i64,
) -> Result<(), String> {
    if canonical_article_id == duplicate_article_id {
        return Ok(());
    }

    transaction
        .execute(
            "
            INSERT OR IGNORE INTO related_articles
                (canonical_article_id, feed_id, title, summary, url, published_at, fetched_at)
            SELECT ?1, feed_id, title, summary, url, published_at, fetched_at
            FROM articles
            WHERE id = ?2
            ",
            params![canonical_article_id, duplicate_article_id],
        )
        .map_err(|err| err.to_string())?;
    transaction
        .execute(
            "UPDATE related_articles SET canonical_article_id = ?1 WHERE canonical_article_id = ?2",
            params![canonical_article_id, duplicate_article_id],
        )
        .map_err(|err| err.to_string())?;
    transaction
        .execute(
            "
            INSERT OR IGNORE INTO article_feeds (article_id, feed_id)
            SELECT ?1, feed_id FROM article_feeds WHERE article_id = ?2
            ",
            params![canonical_article_id, duplicate_article_id],
        )
        .map_err(|err| err.to_string())?;
    transaction
        .execute(
            "
            UPDATE articles
            SET is_read = MAX(
                is_read,
                COALESCE((SELECT is_read FROM articles WHERE id = ?2), 0)
            )
            WHERE id = ?1
            ",
            params![canonical_article_id, duplicate_article_id],
        )
        .map_err(|err| err.to_string())?;

    merge_classifications(transaction, canonical_article_id, duplicate_article_id)?;
    merge_read_events(transaction, canonical_article_id, duplicate_article_id)?;

    transaction
        .execute(
            "DELETE FROM articles WHERE id = ?1",
            params![duplicate_article_id],
        )
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn merge_classifications(
    transaction: &Transaction<'_>,
    canonical_article_id: i64,
    duplicate_article_id: i64,
) -> Result<(), String> {
    let classifications = {
        let mut stmt = transaction
            .prepare(
                "
                SELECT zone_id, relevant, confidence, match_level, classified_at
                FROM zone_articles
                WHERE article_id = ?1
                ",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(params![duplicate_article_id], |row| {
                Ok(StoredClassification {
                    zone_id: row.get(0)?,
                    relevant: row.get(1)?,
                    confidence: row.get(2)?,
                    match_level: row.get(3)?,
                    classified_at: row.get(4)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        rows
    };

    for classification in classifications {
        let existing_level = transaction
            .query_row(
                "SELECT match_level FROM zone_articles WHERE zone_id = ?1 AND article_id = ?2",
                params![classification.zone_id, canonical_article_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;
        match existing_level {
            Some(existing_level)
                if match_level_rank(&classification.match_level)
                    > match_level_rank(&existing_level) =>
            {
                transaction
                    .execute(
                        "
                        UPDATE zone_articles
                        SET relevant = ?1, confidence = ?2, match_level = ?3, classified_at = ?4
                        WHERE zone_id = ?5 AND article_id = ?6
                        ",
                        params![
                            classification.relevant,
                            classification.confidence,
                            classification.match_level,
                            classification.classified_at,
                            classification.zone_id,
                            canonical_article_id
                        ],
                    )
                    .map_err(|err| err.to_string())?;
            }
            Some(_) => {}
            None => {
                transaction
                    .execute(
                        "
                        INSERT INTO zone_articles
                            (zone_id, article_id, relevant, confidence, match_level, classified_at)
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                        ",
                        params![
                            classification.zone_id,
                            canonical_article_id,
                            classification.relevant,
                            classification.confidence,
                            classification.match_level,
                            classification.classified_at
                        ],
                    )
                    .map_err(|err| err.to_string())?;
            }
        }
    }
    Ok(())
}

fn merge_read_events(
    transaction: &Transaction<'_>,
    canonical_article_id: i64,
    duplicate_article_id: i64,
) -> Result<(), String> {
    let events = {
        let mut stmt = transaction
            .prepare(
                "
                SELECT id, zone_id, opened_at, processed_at
                FROM zone_read_events
                WHERE article_id = ?1
                ",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(params![duplicate_article_id], |row| {
                Ok(StoredReadEvent {
                    id: row.get(0)?,
                    zone_id: row.get(1)?,
                    opened_at: row.get(2)?,
                    processed_at: row.get(3)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        rows
    };

    for event in events {
        let existing = transaction
            .query_row(
                "
                SELECT id, opened_at, processed_at
                FROM zone_read_events
                WHERE zone_id = ?1 AND article_id = ?2
                ",
                params![event.zone_id, canonical_article_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|err| err.to_string())?;

        if let Some((existing_id, existing_opened_at, existing_processed_at)) = existing {
            let opened_at = existing_opened_at.min(event.opened_at);
            let processed_at = existing_processed_at.or(event.processed_at);
            transaction
                .execute(
                    "UPDATE zone_read_events SET opened_at = ?1, processed_at = ?2 WHERE id = ?3",
                    params![opened_at, processed_at, existing_id],
                )
                .map_err(|err| err.to_string())?;
            transaction
                .execute(
                    "DELETE FROM zone_read_events WHERE id = ?1",
                    params![event.id],
                )
                .map_err(|err| err.to_string())?;
        } else {
            transaction
                .execute(
                    "UPDATE zone_read_events SET article_id = ?1 WHERE id = ?2",
                    params![canonical_article_id, event.id],
                )
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

fn match_level_rank(match_level: &str) -> u8 {
    match match_level {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn insert_related_article(
    conn: &Connection,
    canonical_article_id: i64,
    feed_id: i64,
    article: &NewArticle,
) -> Result<(), String> {
    conn.execute(
        "
        INSERT OR IGNORE INTO related_articles
            (canonical_article_id, feed_id, title, summary, url, published_at, fetched_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
        params![
            canonical_article_id,
            feed_id,
            article.title,
            article.summary,
            article.url,
            article.published_at,
            Utc::now().to_rfc3339()
        ],
    )
    .map(|_| ())
    .map_err(|err| err.to_string())
}

fn article_index_cutoff() -> String {
    (Utc::now() - Duration::seconds(INDEX_RETENTION_SECONDS)).to_rfc3339()
}

fn article_index_cutoff_timestamp() -> i64 {
    (Utc::now() - Duration::seconds(INDEX_RETENTION_SECONDS)).timestamp()
}

fn load_existing_index_entries(
    conn: &Connection,
    cutoff: &str,
) -> Result<Vec<IndexedArticle>, String> {
    let mut stmt = conn
        .prepare(
            "
            SELECT id, title, summary, COALESCE(published_at, fetched_at)
            FROM articles
            WHERE datetime(COALESCE(published_at, fetched_at)) >= datetime(?1)
            ORDER BY id
            ",
        )
        .map_err(|err| err.to_string())?;
    let articles = stmt
        .query_map(params![cutoff], |row| {
            Ok(IndexedArticle {
                article_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                event_at: row.get(3)?,
            })
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    Ok(articles)
}

fn visible_zone_ids_for_article(
    transaction: &Transaction<'_>,
    article_id: i64,
    direct_zone_id: Option<i64>,
) -> Result<Vec<i64>, String> {
    let mut stmt = transaction
        .prepare(
            "
            SELECT zones.id
            FROM zones
            INNER JOIN zone_articles
                ON zone_articles.zone_id = zones.id
                AND zone_articles.article_id = ?1
            WHERE (?2 IS NULL OR zones.id = ?2)
                AND CASE zone_articles.match_level
                        WHEN 'high' THEN 3
                        WHEN 'medium' THEN 2
                        WHEN 'low' THEN 1
                        ELSE 0
                    END >= CASE zones.focus_level
                        WHEN 'laser' THEN 3
                        WHEN 'focused' THEN 2
                        ELSE 1
                    END
                AND EXISTS (
                    SELECT 1
                    FROM article_feeds
                    INNER JOIN zone_feeds
                        ON zone_feeds.feed_id = article_feeds.feed_id
                        AND zone_feeds.zone_id = zones.id
                    WHERE article_feeds.article_id = ?1
                )
            ORDER BY zones.id
            ",
        )
        .map_err(|err| err.to_string())?;
    let zone_ids = stmt
        .query_map(params![article_id, direct_zone_id], |row| row.get(0))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    Ok(zone_ids)
}

fn reset_zone_behavior_in_transaction(
    transaction: &Transaction<'_>,
    zone_id: i64,
) -> Result<(), String> {
    let updated = transaction
        .execute(
            "
            UPDATE zone_behavior_profiles
            SET profile = '',
                processed_reads = 0,
                revision = revision + 1,
                updated_at = NULL,
                last_error = NULL
            WHERE zone_id = ?1
            ",
            params![zone_id],
        )
        .map_err(|err| err.to_string())?;
    if updated == 0 {
        return Err("Xone not found".to_string());
    }
    transaction
        .execute(
            "DELETE FROM zone_read_events WHERE zone_id = ?1",
            params![zone_id],
        )
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn row_to_feed(row: &rusqlite::Row<'_>) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        site_url: row.get(4)?,
        category: row.get(5)?,
        region: row.get(6)?,
        last_fetched_at: row.get(7)?,
        created_at: row.get(8)?,
        article_count: row.get(9)?,
    })
}

fn row_to_article(row: &rusqlite::Row<'_>) -> rusqlite::Result<Article> {
    let is_read: i64 = row.get(8)?;
    Ok(Article {
        id: row.get(0)?,
        feed_id: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        summary: row.get(4)?,
        url: row.get(5)?,
        author: row.get(6)?,
        published_at: row.get(7)?,
        is_read: is_read != 0,
        fetched_at: row.get(9)?,
        ai_match_level: row.get(10)?,
        related_articles: Vec::new(),
    })
}

fn load_related_articles(conn: &Connection, articles: &mut [Article]) -> Result<(), String> {
    if articles.is_empty() {
        return Ok(());
    }

    let article_ids = articles
        .iter()
        .map(|article| article.id)
        .collect::<Vec<_>>();
    let mut related_by_article: HashMap<i64, Vec<RelatedArticle>> = HashMap::new();
    for article_id_chunk in article_ids.chunks(500) {
        let placeholders = std::iter::repeat("?")
            .take(article_id_chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = conn
            .prepare(&format!(
                "
                SELECT canonical_article_id, feed_id, title, summary, url, published_at,
                    fetched_at
                FROM related_articles
                WHERE canonical_article_id IN ({placeholders})
                ORDER BY datetime(COALESCE(published_at, fetched_at)) DESC, id DESC
                "
            ))
            .map_err(|err| err.to_string())?;
        let related = stmt
            .query_map(rusqlite::params_from_iter(article_id_chunk), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    RelatedArticle {
                        feed_id: row.get(1)?,
                        title: row.get(2)?,
                        summary: row.get(3)?,
                        url: row.get(4)?,
                        published_at: row.get(5)?,
                        fetched_at: row.get(6)?,
                    },
                ))
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;

        for (article_id, related_article) in related {
            related_by_article
                .entry(article_id)
                .or_default()
                .push(related_article);
        }
    }
    for article in articles {
        article.related_articles = related_by_article.remove(&article.id).unwrap_or_default();
    }
    Ok(())
}

fn row_to_zone(row: &rusqlite::Row<'_>) -> rusqlite::Result<Zone> {
    Ok(Zone {
        id: row.get(0)?,
        name: row.get(1)?,
        filter_description: row.get(2)?,
        focus_level: row.get(3)?,
        created_at: row.get(4)?,
        unread_count: row.get(5)?,
        last_updated_at: row.get(6)?,
        behavior_profile: row.get(7)?,
        behavior_reads_processed: row.get(8)?,
        behavior_reads_pending: row.get(9)?,
        behavior_updated_at: row.get(10)?,
        behavior_last_error: row.get(11)?,
    })
}

fn validate_focus_level(focus_level: &str) -> Result<(), String> {
    match focus_level {
        "broad" | "focused" | "laser" => Ok(()),
        _ => Err("Invalid Xone focus level".to_string()),
    }
}

fn row_to_setting(row: &rusqlite::Row<'_>) -> rusqlite::Result<Setting> {
    Ok(Setting {
        key: row.get(0)?,
        value: row.get(1)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn fresh_install_seeds_only_the_three_public_xones() {
        let db = Db::new(Path::new(":memory:")).expect("create database");
        assert_eq!(db.list_feeds().unwrap().len(), 494);
        db.seed_default_zones().unwrap();
        let zones = db.list_zones().unwrap();
        assert_eq!(zones.iter().map(|zone| zone.name.as_str()).collect::<Vec<_>>(),
                   vec!["AI", "Open Source AI", "Design"]);
        for (zone, expected) in zones.iter().zip(defaults::DEFAULT_ZONES) {
            assert_eq!(zone.filter_description, expected.filter);
            assert_eq!(zone.focus_level, expected.focus);
            let urls = db.get_zone_feeds(zone.id).unwrap().into_iter()
                .map(|feed| feed.url).collect::<std::collections::HashSet<_>>();
            assert_eq!(urls.len(), 8);
            assert!(expected.feed_urls.iter().all(|url| urls.contains(*url)));
        }
        db.seed_default_zones().unwrap();
        assert_eq!(db.list_zones().unwrap().len(), 3);
        let conn = db.conn.lock().unwrap();
        for table in ["articles", "zone_read_events", "zone_summaries", "zone_articles"] {
            assert_eq!(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        }
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM settings WHERE lower(key) LIKE '%key%' AND value <> ''", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn starter_xones_leave_existing_users_unchanged() {
        let db = Db::new(Path::new(":memory:")).unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT INTO zones (name, filter_description, focus_level, display_order, created_at) VALUES ('Personal', 'My brief', 'laser', 0, '2026-10-04')", []
        ).unwrap();
        db.seed_default_zones().unwrap();
        let zones = db.list_zones().unwrap();
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].name, "Personal");
    }

    #[test]
    fn existing_database_gains_region_and_article_feed_membership() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "
            CREATE TABLE feeds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                description TEXT,
                site_url TEXT,
                category TEXT,
                last_fetched_at TEXT,
                created_at TEXT NOT NULL
            );
            INSERT INTO feeds (url, title, category, created_at)
            VALUES ('https://example.com/old.xml', 'Existing Feed', 'News', '2026-01-01');

            CREATE TABLE articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                normalized_title TEXT NOT NULL,
                content TEXT NOT NULL,
                summary TEXT,
                url TEXT NOT NULL UNIQUE,
                author TEXT,
                published_at TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                fetched_at TEXT NOT NULL,
                FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE CASCADE
            );
            INSERT INTO articles
                (feed_id, title, normalized_title, content, url, fetched_at)
            VALUES
                (1, 'Existing Article', 'existing article', 'Body',
                 'https://example.com/article', '2026-01-01');

            CREATE TABLE zones (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                filter_description TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            INSERT INTO zones (name, filter_description, created_at)
            VALUES
                ('Existing Xone', 'existing filter', '2026-01-01'),
                ('Alpha Xone', 'alphabetical filter', '2026-01-02');

            CREATE TABLE zone_articles (
                zone_id INTEGER NOT NULL,
                article_id INTEGER NOT NULL,
                relevant INTEGER NOT NULL,
                confidence REAL NOT NULL,
                classified_at TEXT NOT NULL,
                PRIMARY KEY (zone_id, article_id)
            );
            INSERT INTO zone_articles
                (zone_id, article_id, relevant, confidence, classified_at)
            VALUES (1, 1, 1, 0.0, '2026-01-01');
            ",
        )
        .expect("create legacy feed table");
        let db = Db {
            conn: Mutex::new(conn),
            article_index: Mutex::new(ArticleIndex::default()),
        };

        db.init().expect("migrate database");

        let conn = db.conn.lock().expect("lock database");
        let region: Option<String> = conn
            .query_row(
                "SELECT region FROM feeds WHERE url = 'https://example.com/old.xml'",
                [],
                |row| row.get(0),
            )
            .expect("load migrated feed");
        assert_eq!(region, None);
        let membership_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM article_feeds WHERE article_id = 1 AND feed_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count migrated article membership");
        assert_eq!(membership_count, 1);
        let (focus_level, match_level): (String, String) = conn
            .query_row(
                "SELECT zones.focus_level, zone_articles.match_level
                 FROM zones
                 INNER JOIN zone_articles ON zone_articles.zone_id = zones.id
                 WHERE zones.id = 1 AND zone_articles.article_id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load migrated match settings");
        assert_eq!(focus_level, "focused");
        assert_eq!(match_level, "no");
        drop(conn);
        assert_eq!(
            db.list_zones()
                .expect("list migrated zones")
                .into_iter()
                .map(|zone| zone.name)
                .collect::<Vec<_>>(),
            vec!["Alpha Xone", "Existing Xone"]
        );
    }

    #[test]
    fn zones_report_latest_successful_fetch_from_assigned_feeds() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let first_feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/first-zone-feed.xml".to_string(),
                    title: "First Zone Feed".to_string(),
                    description: None,
                    site_url: Some("https://example.com".to_string()),
                },
                Some("Technology"),
                None,
            )
            .expect("insert first feed");
        let second_feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/second-zone-feed.xml".to_string(),
                    title: "Second Zone Feed".to_string(),
                    description: None,
                    site_url: Some("https://example.com".to_string()),
                },
                Some("Technology"),
                None,
            )
            .expect("insert second feed");
        let zone_id = db
            .create_zone("Updated Zone", "updated sources", "focused")
            .expect("create updated zone");
        let never_updated_zone_id = db
            .create_zone("Never Updated Zone", "no assigned sources", "focused")
            .expect("create empty zone");
        db.assign_feed(zone_id, first_feed_id)
            .expect("assign first feed");
        db.assign_feed(zone_id, second_feed_id)
            .expect("assign second feed");

        {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "UPDATE feeds SET last_fetched_at = ?1 WHERE id = ?2",
                params!["2026-07-22T10:00:00+00:00", first_feed_id],
            )
            .expect("set first fetch time");
            conn.execute(
                "UPDATE feeds SET last_fetched_at = ?1 WHERE id = ?2",
                params!["2026-07-23T12:30:00+00:00", second_feed_id],
            )
            .expect("set second fetch time");
        }

        let listed_zones = db.list_zones().expect("list zones");
        assert_eq!(
            listed_zones
                .iter()
                .find(|zone| zone.id == zone_id)
                .and_then(|zone| zone.last_updated_at.as_deref()),
            Some("2026-07-23T12:30:00+00:00")
        );
        assert_eq!(
            listed_zones
                .iter()
                .find(|zone| zone.id == never_updated_zone_id)
                .and_then(|zone| zone.last_updated_at.as_deref()),
            None
        );
        assert_eq!(
            db.get_zone(zone_id)
                .expect("get zone")
                .and_then(|zone| zone.last_updated_at),
            Some("2026-07-23T12:30:00+00:00".to_string())
        );
        assert_eq!(
            db.list_zones_for_feed(first_feed_id)
                .expect("list zones for feed")
                .first()
                .and_then(|zone| zone.last_updated_at.as_deref()),
            Some("2026-07-23T12:30:00+00:00")
        );
    }

    #[test]
    fn shared_articles_report_the_source_the_caller_asked_about() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let unassigned_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/attribution-first.xml".to_string(),
                    title: "First Publisher".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert first feed");
        let assigned_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/attribution-second.xml".to_string(),
                    title: "Second Publisher".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("AI"),
                None,
            )
            .expect("insert second feed");
        let article = NewArticle {
            title: "Shared attribution story".to_string(),
            content: "Shared body".to_string(),
            summary: None,
            url: "https://example.com/shared-attribution".to_string(),
            author: None,
            published_at: Some(Utc::now().to_rfc3339()),
        };

        // The first feed stores the canonical row; the second only joins it.
        db.insert_articles(unassigned_feed, std::slice::from_ref(&article))
            .expect("store canonical article");
        db.insert_articles(assigned_feed, std::slice::from_ref(&article))
            .expect("share article with second feed");

        assert_eq!(
            db.list_articles(assigned_feed).expect("list by feed")[0].feed_id,
            assigned_feed,
            "a source page must attribute articles to that source"
        );
        assert_eq!(
            db.list_articles(unassigned_feed).expect("list by feed")[0].feed_id,
            unassigned_feed
        );

        let zone_id = db
            .create_zone("Attribution", "shared attribution", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, assigned_feed)
            .expect("assign only the second feed");
        let article_id = db.list_articles(assigned_feed).expect("list by feed")[0].id;
        db.save_classification(zone_id, article_id, "high")
            .expect("classify shared article");

        assert_eq!(
            db.list_zone_articles(zone_id).expect("list zone articles")[0].feed_id,
            assigned_feed,
            "a Xone must not attribute an article to a source it does not include"
        );
    }

    #[test]
    fn saved_publisher_names_survive_re_adding_a_source() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let metadata = FeedMetadata {
            url: "https://example.com/renamed.xml".to_string(),
            title: "Curated Publisher Name".to_string(),
            description: None,
            site_url: None,
        };
        db.upsert_feed(&metadata, Some("Technology"), None)
            .expect("insert feed");

        assert_eq!(
            db.feed_title(&metadata.url).expect("load saved title"),
            Some("Curated Publisher Name".to_string())
        );
        assert_eq!(
            db.feed_title("https://example.com/missing.xml")
                .expect("load missing title"),
            None
        );
    }

    #[test]
    fn zone_articles_follow_assigned_feeds() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/feed.xml".to_string(),
                    title: "Example Feed".to_string(),
                    description: None,
                    site_url: Some("https://example.com".to_string()),
                },
                Some("Technology"),
                Some("Global"),
            )
            .expect("insert feed");
        assert_eq!(
            db.get_feed(feed_id).expect("load feed").unwrap().category,
            Some("Technology".to_string())
        );
        assert_eq!(
            db.get_feed(feed_id).expect("load feed").unwrap().region,
            Some("Global".to_string())
        );
        db.insert_articles(
            feed_id,
            &[NewArticle {
                title: "Example zone article".to_string(),
                content: "Clean article body".to_string(),
                summary: None,
                url: "https://example.com/article".to_string(),
                author: None,
                published_at: Some("2026-01-01T00:00:00Z".to_string()),
            }],
        )
        .expect("insert article");

        let zone_id = db
            .create_zone("Tech", "only technology news", "focused")
            .expect("create zone");
        assert!(db
            .list_zone_articles(zone_id)
            .expect("list before assign")
            .is_empty());

        db.assign_feed(zone_id, feed_id).expect("assign feed");
        let pending = db
            .list_zone_articles_for_classification(zone_id, false, None)
            .expect("list pending classification");
        assert_eq!(pending.len(), 1);
        db.save_classification(zone_id, pending[0].id, "low")
            .expect("save classification");

        assert!(db
            .list_zone_articles(zone_id)
            .expect("list focused matches")
            .is_empty());

        db.update_zone(zone_id, "Tech", "only technology news", "broad")
            .expect("switch to broad focus");
        let articles = db.list_zone_articles(zone_id).expect("list broad matches");
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title, "Example zone article");
        assert_eq!(articles[0].ai_match_level.as_deref(), Some("low"));

        db.update_zone(zone_id, "Tech", "only technology news", "laser")
            .expect("switch to laser focus");
        assert!(db
            .list_zone_articles(zone_id)
            .expect("list laser matches")
            .is_empty());
        db.save_classification(zone_id, pending[0].id, "high")
            .expect("save high classification");
        assert_eq!(
            db.list_zone_articles(zone_id)
                .expect("list high matches")
                .first()
                .and_then(|article| article.ai_match_level.as_deref()),
            Some("high")
        );

        db.unassign_feed(zone_id, feed_id).expect("unassign feed");
        assert!(db
            .list_zone_articles(zone_id)
            .expect("list after unassign")
            .is_empty());
    }

    #[test]
    fn classification_candidates_can_be_limited_to_a_recent_window() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/recent.xml".to_string(),
                    title: "Recent Feed".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");
        db.insert_articles(
            feed_id,
            &[
                NewArticle {
                    title: "Recent classification candidate".to_string(),
                    content: "Recent body".to_string(),
                    summary: None,
                    url: "https://example.com/recent".to_string(),
                    author: None,
                    published_at: Some("2026-08-20T00:30:00Z".to_string()),
                },
                NewArticle {
                    title: "Older classification candidate".to_string(),
                    content: "Older body".to_string(),
                    summary: None,
                    url: "https://example.com/older".to_string(),
                    author: None,
                    published_at: Some("2026-08-20T01:00:00+02:00".to_string()),
                },
            ],
        )
        .expect("insert articles");
        let zone_id = db
            .create_zone("Recent", "recent technology", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, feed_id).expect("assign feed");

        let candidates = db
            .list_zone_articles_for_classification(zone_id, false, Some("2026-08-20T00:00:00Z"))
            .expect("list recent candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "Recent classification candidate");
    }

    #[test]
    fn duplicate_article_can_belong_to_multiple_feeds_without_duplicate_zone_rows() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let first_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/general.xml".to_string(),
                    title: "General".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("General News"),
                Some("World"),
            )
            .expect("insert first feed");
        let second_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/ai.xml".to_string(),
                    title: "AI".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("AI"),
                None,
            )
            .expect("insert second feed");
        let article = NewArticle {
            title: "Shared artificial intelligence story".to_string(),
            content: "Shared story body".to_string(),
            summary: None,
            url: "https://example.com/shared-story".to_string(),
            author: None,
            published_at: Some("2026-07-22T00:00:00Z".to_string()),
        };

        assert_eq!(
            db.insert_articles(first_feed, std::slice::from_ref(&article)),
            Ok(1)
        );
        assert_eq!(
            db.insert_articles(second_feed, std::slice::from_ref(&article)),
            Ok(1)
        );
        assert_eq!(
            db.list_articles(first_feed).expect("list first feed").len(),
            1
        );
        assert_eq!(
            db.list_articles(second_feed)
                .expect("list second feed")
                .len(),
            1
        );

        let zone_id = db
            .create_zone("AI", "only artificial intelligence news", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, first_feed)
            .expect("assign first feed");
        db.assign_feed(zone_id, second_feed)
            .expect("assign second feed");
        assert_eq!(
            db.list_zone_articles_for_classification(zone_id, false, None)
                .expect("list classification candidates")
                .len(),
            1
        );

        db.remove_feed(first_feed).expect("remove canonical feed");
        assert_eq!(
            db.list_articles(second_feed)
                .expect("list surviving feed")
                .len(),
            1
        );
    }

    #[test]
    fn indexed_dedup_handles_same_batch_and_feed_removal() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let first_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/indexed-first.xml".to_string(),
                    title: "Indexed First".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert first feed");
        let articles = [
            NewArticle {
                title: "Frontier model safety evaluation published".to_string(),
                content: "First body".to_string(),
                summary: None,
                url: "https://example.com/indexed-first".to_string(),
                author: None,
                published_at: Some("2026-07-22T00:00:00Z".to_string()),
            },
            NewArticle {
                title: "Frontier model safety evaluation results".to_string(),
                content: "Duplicate body".to_string(),
                summary: None,
                url: "https://example.com/indexed-duplicate".to_string(),
                author: None,
                published_at: Some("2026-07-22T00:00:00Z".to_string()),
            },
        ];

        assert_eq!(db.insert_articles(first_feed, &articles), Ok(1));
        let clustered = db.list_articles(first_feed).expect("list first");
        assert_eq!(clustered.len(), 1);
        assert_eq!(clustered[0].related_articles.len(), 1);
        assert_eq!(
            clustered[0].related_articles[0].url,
            "https://example.com/indexed-duplicate"
        );
        assert_eq!(db.insert_articles(first_feed, &articles), Ok(0));
        assert_eq!(
            db.list_articles(first_feed).expect("list cluster again")[0]
                .related_articles
                .len(),
            1,
            "refreshing the same feed must not duplicate cluster members"
        );
        let zone_id = db
            .create_zone("Cluster", "frontier model safety", "focused")
            .expect("create cluster zone");
        db.assign_feed(zone_id, first_feed)
            .expect("assign clustered feed");
        assert_eq!(
            db.list_zone_articles_for_classification(zone_id, false, None)
                .expect("list cluster classification candidates")
                .len(),
            1,
            "only the canonical article should require classification"
        );
        db.remove_feed(first_feed).expect("remove first feed");

        let second_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/indexed-second.xml".to_string(),
                    title: "Indexed Second".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert second feed");
        assert_eq!(
            db.insert_articles(second_feed, &articles[1..]),
            Ok(1),
            "removing a source must also remove its titles from the index"
        );
    }

    #[test]
    fn recent_cluster_reconciliation_preserves_article_state_and_is_idempotent() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let first_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/reconcile-first.xml".to_string(),
                    title: "Reconcile First".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert first feed");
        let second_feed = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/reconcile-second.xml".to_string(),
                    title: "Reconcile Second".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert second feed");
        let zone_id = db
            .create_zone("AI Reconcile", "AI acquisitions", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, first_feed)
            .expect("assign first feed");
        db.assign_feed(zone_id, second_feed)
            .expect("assign second feed");

        let (first_article, read_article) = {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "
                INSERT INTO articles
                    (feed_id, title, normalized_title, content, summary, url, published_at,
                     is_read, fetched_at)
                VALUES (?1, ?2, ?3, 'Body', ?4, ?5, ?6, 0, ?6)
                ",
                params![
                    first_feed,
                    "Nvidia closes in on Hugging Face acquisition",
                    "nvidia closes in on hugging face acquisition",
                    "Nvidia reportedly agreed to buy Hugging Face for $12.9 billion.",
                    "https://example.com/reconcile-first",
                    "2026-08-27T05:00:00Z"
                ],
            )
            .expect("insert first canonical");
            let first_article = conn.last_insert_rowid();
            conn.execute(
                "
                INSERT INTO articles
                    (feed_id, title, normalized_title, content, summary, url, published_at,
                     is_read, fetched_at)
                VALUES (?1, ?2, ?3, 'Body', ?4, ?5, ?6, 1, ?6)
                ",
                params![
                    second_feed,
                    "Nvidia agrees to acquire Hugging Face for $13B",
                    "nvidia agrees to acquire hugging face for 13b",
                    "The chipmaker agreed to buy the open-source AI hub.",
                    "https://example.com/reconcile-second",
                    "2026-08-27T06:00:00Z"
                ],
            )
            .expect("insert read canonical");
            let read_article = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO article_feeds (article_id, feed_id) VALUES (?1, ?2)",
                params![first_article, first_feed],
            )
            .expect("associate first article");
            conn.execute(
                "INSERT INTO article_feeds (article_id, feed_id) VALUES (?1, ?2)",
                params![read_article, second_feed],
            )
            .expect("associate read article");
            conn.execute(
                "
                INSERT INTO zone_articles
                    (zone_id, article_id, relevant, confidence, match_level, classified_at)
                VALUES (?1, ?2, 1, 0.7, 'medium', '2026-08-27T06:05:00Z'),
                       (?1, ?3, 1, 1.0, 'high', '2026-08-27T06:06:00Z')
                ",
                params![zone_id, first_article, read_article],
            )
            .expect("classify separate canonicals");
            conn.execute(
                "
                INSERT INTO zone_read_events (zone_id, article_id, opened_at, processed_at)
                VALUES (?1, ?2, '2026-08-27T06:10:00Z', NULL)
                ",
                params![zone_id, read_article],
            )
            .expect("insert learning event");
            (first_article, read_article)
        };

        assert_eq!(
            db.reconcile_article_clusters_since("2026-08-27T00:00:00Z"),
            Ok(1)
        );
        let conn = db.conn.lock().expect("lock reconciled database");
        let canonical: (i64, bool) = conn
            .query_row(
                "SELECT id, is_read FROM articles WHERE id IN (?1, ?2)",
                params![first_article, read_article],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load surviving canonical");
        assert_eq!(canonical, (read_article, true));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM related_articles WHERE canonical_article_id = ?1",
                params![read_article],
                |row| row.get::<_, i64>(0),
            )
            .expect("count related reports"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM article_feeds WHERE article_id = ?1",
                params![read_article],
                |row| row.get::<_, i64>(0),
            )
            .expect("count preserved memberships"),
            2
        );
        assert_eq!(
            conn.query_row(
                "SELECT match_level FROM zone_articles WHERE zone_id = ?1 AND article_id = ?2",
                params![zone_id, read_article],
                |row| row.get::<_, String>(0),
            )
            .expect("load strongest classification"),
            "high"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM zone_read_events WHERE zone_id = ?1 AND article_id = ?2",
                params![zone_id, read_article],
                |row| row.get::<_, i64>(0),
            )
            .expect("count preserved learning event"),
            1
        );
        drop(conn);

        assert_eq!(
            db.reconcile_article_clusters_since("2026-08-27T00:00:00Z"),
            Ok(0)
        );
        let conn = db.conn.lock().expect("lock idempotent database");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM related_articles", [], |row| row
                .get::<_, i64>(0))
                .expect("count related reports after second pass"),
            1
        );
    }

    #[test]
    fn smart_feeds_only_include_articles_visible_in_zones() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let assigned_feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/assigned-smart.xml".to_string(),
                    title: "Assigned Smart Feed Test".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert assigned feed");
        let unassigned_feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/unassigned-smart.xml".to_string(),
                    title: "Unassigned Smart Feed Test".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Business"),
                None,
            )
            .expect("insert unassigned feed");
        let now = Utc::now();
        db.insert_articles(
            assigned_feed_id,
            &[
                NewArticle {
                    title: "Current technology briefing".to_string(),
                    content: "Current article body".to_string(),
                    summary: None,
                    url: "https://example.com/current-smart-article".to_string(),
                    author: None,
                    published_at: Some(now.to_rfc3339()),
                },
                NewArticle {
                    title: "Earlier finance bulletin".to_string(),
                    content: "Older article body".to_string(),
                    summary: None,
                    url: "https://example.com/older-smart-article".to_string(),
                    author: None,
                    published_at: Some((now - chrono::Duration::hours(48)).to_rfc3339()),
                },
                NewArticle {
                    title: "Local weather advisory".to_string(),
                    content: "Weather article body".to_string(),
                    summary: None,
                    url: "https://example.com/irrelevant-smart-article".to_string(),
                    author: None,
                    published_at: Some(now.to_rfc3339()),
                },
            ],
        )
        .expect("insert assigned-feed articles");
        db.insert_articles(
            unassigned_feed_id,
            &[NewArticle {
                title: "International sports roundup".to_string(),
                content: "Sports article body".to_string(),
                summary: None,
                url: "https://example.com/unassigned-smart-article".to_string(),
                author: None,
                published_at: Some(now.to_rfc3339()),
            }],
        )
        .expect("insert unassigned-feed article");

        assert_eq!(db.smart_feed_counts().expect("count before zone").today, 0);

        let zone_id = db
            .create_zone("Smart", "only selected smart articles", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, assigned_feed_id)
            .expect("assign feed to zone");

        let assigned_articles = db
            .list_articles(assigned_feed_id)
            .expect("list assigned-feed articles");
        for article in &assigned_articles {
            let match_level = if article.url == "https://example.com/irrelevant-smart-article" {
                "no"
            } else {
                "high"
            };
            db.save_classification(zone_id, article.id, match_level)
                .expect("classify assigned-feed article");
        }
        let unassigned_article = db
            .list_articles(unassigned_feed_id)
            .expect("list unassigned-feed articles")
            .into_iter()
            .next()
            .expect("load unassigned-feed article");
        db.save_classification(zone_id, unassigned_article.id, "high")
            .expect("save stale unassigned classification");

        let counts = db.smart_feed_counts().expect("count smart feeds");
        assert_eq!(counts.today, 1);
        assert_eq!(counts.all_unread, 2);
        let today = db.list_today_articles().expect("list today");
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].url, "https://example.com/current-smart-article");
        assert_eq!(db.list_unread_articles().expect("list unread").len(), 2);

        let today_id = today[0].id;
        db.record_article_open(today_id, None, false, 10)
            .expect("mark current article read");

        let counts = db.smart_feed_counts().expect("recount smart feeds");
        assert_eq!(counts.today, 1);
        assert_eq!(counts.all_unread, 1);
        assert_eq!(db.list_unread_articles().expect("reload unread").len(), 1);

        db.unassign_feed(zone_id, assigned_feed_id)
            .expect("unassign feed from zone");
        let counts = db.smart_feed_counts().expect("count after unassign");
        assert_eq!(counts.today, 0);
        assert_eq!(counts.all_unread, 0);
    }

    #[test]
    fn retired_default_feed_merges_preserve_articles_and_zone_assignments() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let retained_url = "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml";
        let retired_url = "https://www.theverge.com/rss/index.xml";
        let (retained_id, retired_id) = {
            let conn = db.conn.lock().expect("lock database");
            let retained_id = conn
                .query_row(
                    "SELECT id FROM feeds WHERE url = ?1",
                    params![retained_url],
                    |row| row.get::<_, i64>(0),
                )
                .expect("load retained feed");
            conn.execute(
                "
                INSERT INTO feeds (url, title, category, region, created_at)
                VALUES (?1, 'The Verge', 'Tech', NULL, ?2)
                ",
                params![retired_url, Utc::now().to_rfc3339()],
            )
            .expect("insert retired feed");
            (retained_id, conn.last_insert_rowid())
        };

        db.insert_articles(
            retired_id,
            &[NewArticle {
                title: "Retired source article".to_string(),
                content: "Article body".to_string(),
                summary: None,
                url: "https://example.com/retired-source-article".to_string(),
                author: None,
                published_at: Some("2026-07-22T00:00:00Z".to_string()),
            }],
        )
        .expect("insert retired-source article");
        let zone_id = db
            .create_zone("AI", "only artificial intelligence news", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, retired_id)
            .expect("assign retired feed");

        db.seed_default_feeds().expect("merge catalog feeds");

        let conn = db.conn.lock().expect("lock database");
        let retired_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM feeds WHERE url = ?1",
                params![retired_url],
                |row| row.get(0),
            )
            .expect("count retired feeds");
        let article_feed_id: i64 = conn
            .query_row(
                "SELECT feed_id FROM articles WHERE url = 'https://example.com/retired-source-article'",
                [],
                |row| row.get(0),
            )
            .expect("load migrated article");
        let assignment_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_feeds WHERE zone_id = ?1 AND feed_id = ?2",
                params![zone_id, retained_id],
                |row| row.get(0),
            )
            .expect("count migrated assignments");
        let membership_count: i64 = conn
            .query_row(
                "
                SELECT COUNT(*)
                FROM article_feeds
                WHERE article_id = (
                    SELECT id FROM articles
                    WHERE url = 'https://example.com/retired-source-article'
                )
                    AND feed_id = ?1
                ",
                params![retained_id],
                |row| row.get(0),
            )
            .expect("count migrated article membership");

        assert_eq!(retired_count, 0);
        assert_eq!(article_feed_id, retained_id);
        assert_eq!(assignment_count, 1);
        assert_eq!(membership_count, 1);
    }

    #[test]
    fn native_feed_replacement_migrates_a_proxy_and_its_legacy_windows() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let (proxy_url, native_url) = defaults::NATIVE_FEED_REPLACEMENTS[0];
        let legacy_url = proxy_url.replace("when:4d", "when:7d");

        // A database seeded before the swap holds the proxy, and possibly an even older
        // pre-cap window of it, instead of the publisher's own feed.
        let (proxy_id, legacy_id) = {
            let conn = db.conn.lock().expect("lock database");
            conn.execute("DELETE FROM feeds WHERE url = ?1", params![native_url])
                .expect("remove native feed");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO feeds (url, title, category, region, created_at) VALUES (?1, 'Proxy', 'Tech', NULL, ?2)",
                params![proxy_url, now],
            )
            .expect("insert proxy feed");
            let proxy_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO feeds (url, title, category, region, created_at) VALUES (?1, 'Proxy', 'Tech', NULL, ?2)",
                params![legacy_url, now],
            )
            .expect("insert legacy-window feed");
            (proxy_id, conn.last_insert_rowid())
        };

        for (feed_id, slug) in [(proxy_id, "proxy"), (legacy_id, "legacy")] {
            db.insert_articles(
                feed_id,
                &[NewArticle {
                    title: format!("Story from the {slug} source"),
                    content: "Article body".to_string(),
                    summary: None,
                    url: format!("https://example.com/{slug}-story"),
                    author: None,
                    published_at: Some("2026-07-22T00:00:00Z".to_string()),
                }],
            )
            .expect("insert article");
        }
        let zone_id = db
            .create_zone("AI", "only artificial intelligence news", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, proxy_id).expect("assign proxy");
        db.assign_feed(zone_id, legacy_id).expect("assign legacy");

        db.seed_default_feeds().expect("apply native replacement");

        let conn = db.conn.lock().expect("lock database");
        let native_id: i64 = conn
            .query_row(
                "SELECT id FROM feeds WHERE url = ?1",
                params![native_url],
                |row| row.get(0),
            )
            .expect("native feed is seeded");
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM feeds WHERE url IN (?1, ?2)",
                params![proxy_url, legacy_url],
                |row| row.get(0),
            )
            .expect("count leftover proxy rows");
        assert_eq!(leftovers, 0, "proxy rows survived the replacement");

        for slug in ["proxy", "legacy"] {
            let url = format!("https://example.com/{slug}-story");
            let owner: i64 = conn
                .query_row(
                    "SELECT feed_id FROM articles WHERE url = ?1",
                    params![url],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| panic!("{slug} article survived"));
            assert_eq!(owner, native_id, "{slug} article did not follow the feed");
            let membership: i64 = conn
                .query_row(
                    "
                    SELECT COUNT(*) FROM article_feeds
                    WHERE article_id = (SELECT id FROM articles WHERE url = ?1)
                        AND feed_id = ?2
                    ",
                    params![url, native_id],
                    |row| row.get(0),
                )
                .expect("count membership");
            assert_eq!(membership, 1, "{slug} membership did not follow the feed");
        }

        let assignments: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_feeds WHERE zone_id = ?1 AND feed_id = ?2",
                params![zone_id, native_id],
                |row| row.get(0),
            )
            .expect("count migrated assignments");
        assert_eq!(assignments, 1, "Xone assignment did not follow the feed");
    }

    #[test]
    fn restored_catalog_topic_queries_are_preserved_with_custom_sources() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let restored_url = defaults::AUDITED_FEED_RECORDS
            .iter()
            .find(|feed| feed.dedup_key.starts_with("topic-query:"))
            .expect("historical topic query")
            .url;
        let custom_url = "https://news.google.com/rss/search?q=custom+xone+query&hl=en-US";
        {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![TOPIC_QUERY_RETIREMENT_KEY],
            )
            .expect("reset retirement marker");
            conn.execute(
                "
                INSERT INTO feeds (url, title, category, region, created_at)
                VALUES (?1, 'Custom query', 'General News', 'World', ?2)
                ",
                params![custom_url, Utc::now().to_rfc3339()],
            )
            .expect("insert custom query");
        }

        db.seed_default_feeds().expect("reseed current catalog");

        let conn = db.conn.lock().expect("lock database");
        let restored_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM feeds WHERE url = ?1",
                params![restored_url],
                |row| row.get(0),
            )
            .expect("count restored catalog query");
        let custom_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM feeds WHERE url = ?1",
                params![custom_url],
                |row| row.get(0),
            )
            .expect("count custom query");
        let retirement_marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = ?1",
                params![TOPIC_QUERY_RETIREMENT_KEY],
                |row| row.get(0),
            )
            .expect("count retirement marker");

        assert_eq!(restored_count, 1);
        assert_eq!(custom_count, 1);
        assert_eq!(retirement_marker, 1);
    }

    #[test]
    fn refresh_preserves_display_names_and_seeding_repairs_catalog_titles() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let nikkei = db
            .list_feeds()
            .expect("list feeds")
            .into_iter()
            .find(|feed| feed.title == "Nikkei Asia")
            .expect("find Nikkei Asia");
        {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "UPDATE feeds SET title = ?1 WHERE id = ?2",
                params!["site:asia.nikkei.com when:3d - Google News", nikkei.id],
            )
            .expect("replace catalog title");
        }
        db.seed_default_feeds().expect("repair catalog title");
        assert_eq!(
            db.get_feed(nikkei.id)
                .expect("load Nikkei Asia")
                .expect("Nikkei Asia exists")
                .title,
            "Nikkei Asia"
        );

        let custom_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://news.google.com/rss/search?q=site:example.com+when:1d"
                        .to_string(),
                    title: "Example Publisher".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert custom source");
        db.update_feed_fetch(
            custom_id,
            &FeedMetadata {
                url: "https://news.google.com/rss/search?q=site:example.com+when:1d".to_string(),
                title: "site:example.com when:1d - Google News".to_string(),
                description: Some("Updated description".to_string()),
                site_url: Some("https://news.google.com".to_string()),
            },
        )
        .expect("refresh custom source");
        let custom = db
            .get_feed(custom_id)
            .expect("load custom source")
            .expect("custom source exists");
        assert_eq!(custom.title, "Example Publisher");
        assert_eq!(custom.description.as_deref(), Some("Updated description"));
    }

    #[test]
    fn longer_google_news_windows_merge_into_the_four_day_default() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        // Pick whichever publisher-scoped proxy is still seeded, so swapping any single
        // publisher over to its native feed cannot quietly stop exercising this path.
        let retained = db
            .list_feeds()
            .expect("list feeds")
            .into_iter()
            .find(|feed| {
                feed.url.starts_with("https://news.google.com/rss/search?")
                    && feed.url.contains("when:4d")
            })
            .expect("a four-day Google News source is still seeded");
        let retired_url = retained.url.replace("when:4d", "when:7d");
        let retired_id = {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "
                INSERT INTO feeds (url, title, category, region, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    retired_url,
                    retained.title,
                    retained.category,
                    retained.region,
                    Utc::now().to_rfc3339()
                ],
            )
            .expect("insert legacy window");
            conn.last_insert_rowid()
        };
        let zone_id = db
            .create_zone("Middle East", "Middle East publishers", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, retired_id)
            .expect("assign legacy window");

        db.seed_default_feeds().expect("merge legacy window");

        assert!(db
            .list_feeds()
            .expect("list migrated feeds")
            .iter()
            .all(|feed| feed.url != retired_url));
        assert_eq!(
            db.get_zone_feeds(zone_id)
                .expect("list migrated assignments")
                .first()
                .map(|feed| feed.id),
            Some(retained.id)
        );
    }

    #[test]
    fn delete_feed_and_zone_report_missing_rows() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/removable.xml".to_string(),
                    title: "Removable Feed".to_string(),
                    description: None,
                    site_url: Some("https://example.com".to_string()),
                },
                None,
                None,
            )
            .expect("insert feed");
        let zone_id = db
            .create_zone("Removable Zone", "temporary test zone", "focused")
            .expect("create zone");

        db.assign_feed(zone_id, feed_id).expect("assign feed");
        assert_eq!(
            db.get_zone_feeds(zone_id)
                .expect("list assigned feeds")
                .len(),
            1
        );

        db.remove_feed(feed_id).expect("remove feed");
        assert!(!db
            .list_feeds()
            .expect("list feeds")
            .iter()
            .any(|feed| feed.id == feed_id));
        assert!(db
            .get_zone_feeds(zone_id)
            .expect("list assigned feeds after feed removal")
            .is_empty());
        assert!(db.remove_feed(feed_id).is_err());

        db.delete_zone(zone_id).expect("delete zone");
        assert!(!db
            .list_zones()
            .expect("list zones")
            .iter()
            .any(|zone| zone.id == zone_id));
        assert!(db.delete_zone(zone_id).is_err());
    }

    #[test]
    fn zones_can_be_reordered_and_new_zones_append() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let first_id = db
            .create_zone("First", "first filter", "focused")
            .expect("create first zone");
        let second_id = db
            .create_zone("Second", "second filter", "focused")
            .expect("create second zone");
        let third_id = db
            .create_zone("Third", "third filter", "focused")
            .expect("create third zone");

        db.reorder_zones(&[third_id, first_id, second_id])
            .expect("reorder zones");
        assert_eq!(
            db.list_zones()
                .expect("list reordered zones")
                .into_iter()
                .map(|zone| zone.id)
                .collect::<Vec<_>>(),
            vec![third_id, first_id, second_id]
        );

        let fourth_id = db
            .create_zone("Fourth", "fourth filter", "focused")
            .expect("create fourth zone");
        assert_eq!(
            db.list_zones()
                .expect("list zones with appended zone")
                .last()
                .map(|zone| zone.id),
            Some(fourth_id)
        );
        assert!(db.reorder_zones(&[third_id, first_id, second_id]).is_err());
        assert!(db
            .reorder_zones(&[third_id, first_id, first_id, fourth_id])
            .is_err());
    }

    #[test]
    fn article_cleanup_cascades_and_rebuilds_the_article_index() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/retention.xml".to_string(),
                    title: "Retention Feed".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");
        let zone_id = db
            .create_zone("Retention", "retention test", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, feed_id).expect("assign feed");
        db.insert_articles(
            feed_id,
            &[
                NewArticle {
                    title: "Old retained headline".to_string(),
                    content: "Old content".to_string(),
                    summary: None,
                    url: "https://example.com/old-retention-article".to_string(),
                    author: None,
                    published_at: Some("2026-01-01T00:00:00Z".to_string()),
                },
                NewArticle {
                    title: "Unique current technology bulletin".to_string(),
                    content: "Recent content".to_string(),
                    summary: None,
                    url: "https://example.com/recent-retention-article".to_string(),
                    author: None,
                    published_at: Some("2026-02-01T00:00:00Z".to_string()),
                },
            ],
        )
        .expect("insert articles");
        let old_article_id = db
            .list_articles(feed_id)
            .expect("list articles")
            .into_iter()
            .find(|article| article.url.ends_with("old-retention-article"))
            .expect("find old article")
            .id;
        db.save_classification(zone_id, old_article_id, "high")
            .expect("classify old article");
        db.record_article_open(old_article_id, Some(zone_id), false, 10)
            .expect("record old article open");

        assert_eq!(
            db.count_articles_older_than("2026-01-15T00:00:00Z")
                .expect("count old articles"),
            1
        );
        assert_eq!(
            db.delete_articles_older_than("2026-01-15T00:00:00Z")
                .expect("delete old articles"),
            1
        );

        let conn = db.conn.lock().expect("lock database");
        let article_memberships: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM article_feeds WHERE article_id = ?1",
                params![old_article_id],
                |row| row.get(0),
            )
            .expect("count article memberships");
        let classifications: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_articles WHERE article_id = ?1",
                params![old_article_id],
                |row| row.get(0),
            )
            .expect("count classifications");
        let read_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_read_events WHERE article_id = ?1",
                params![old_article_id],
                |row| row.get(0),
            )
            .expect("count read events");
        drop(conn);
        assert_eq!(article_memberships, 0);
        assert_eq!(classifications, 0);
        assert_eq!(read_events, 0);
        assert_eq!(
            db.list_articles(feed_id)
                .expect("list retained articles")
                .len(),
            1
        );

        assert_eq!(
            db.insert_articles(
                feed_id,
                &[NewArticle {
                    title: "Old retained headline".to_string(),
                    content: "Replacement content".to_string(),
                    summary: None,
                    url: "https://example.com/replacement-retention-article".to_string(),
                    author: None,
                    published_at: Some("2026-02-02T00:00:00Z".to_string()),
                }],
            )
            .expect("insert replacement article"),
            1
        );
    }

    #[test]
    fn article_opens_follow_explicit_xone_and_smart_feed_attribution() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/behavior-attribution.xml".to_string(),
                    title: "Behavior Attribution".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");
        let articles = (1..=4)
            .map(|index| NewArticle {
                title: format!("UniqueTopic{index} DistinctAngle{index}"),
                content: format!("Behavior example {index}"),
                summary: None,
                url: format!("https://example.com/behavior-attribution-{index}"),
                author: None,
                published_at: Some(format!("2026-08-21T00:0{index}:00Z")),
            })
            .collect::<Vec<_>>();
        db.insert_articles(feed_id, &articles)
            .expect("insert articles");
        let article_ids = db
            .list_articles(feed_id)
            .expect("list articles")
            .into_iter()
            .map(|article| article.id)
            .collect::<Vec<_>>();
        let first_zone = db
            .create_zone("First", "technology", "focused")
            .expect("create first zone");
        let second_zone = db
            .create_zone("Second", "technology", "focused")
            .expect("create second zone");
        for zone_id in [first_zone, second_zone] {
            db.assign_feed(zone_id, feed_id).expect("assign feed");
            for article_id in &article_ids {
                db.save_classification(zone_id, *article_id, "high")
                    .expect("classify article");
            }
        }

        let direct_open = db
            .record_article_open(article_ids[0], Some(first_zone), false, 10)
            .expect("record direct open");
        assert!(direct_open.newly_read);
        assert_eq!(direct_open.unread_zone_ids, [first_zone, second_zone]);
        assert_eq!(direct_open.behavior_progress.len(), 1);
        assert_eq!(direct_open.behavior_progress[0].zone_id, first_zone);
        assert_eq!(direct_open.behavior_progress[0].pending_reads, 1);
        assert!(direct_open.ready_behavior_zone_ids.is_empty());
        let duplicate_open = db
            .record_article_open(article_ids[0], Some(first_zone), false, 10)
            .expect("record duplicate direct open");
        assert!(!duplicate_open.newly_read);
        assert!(duplicate_open.unread_zone_ids.is_empty());
        assert!(duplicate_open.behavior_progress.is_empty());
        let smart_feed_open = db
            .record_article_open(article_ids[1], None, true, 10)
            .expect("record smart feed open");
        assert_eq!(smart_feed_open.unread_zone_ids, [first_zone, second_zone]);
        assert_eq!(smart_feed_open.behavior_progress.len(), 2);
        assert_eq!(smart_feed_open.behavior_progress[0].pending_reads, 2);
        assert_eq!(smart_feed_open.behavior_progress[1].pending_reads, 1);
        db.unassign_feed(second_zone, feed_id)
            .expect("unassign second zone");
        let smart_feed_open = db
            .record_article_open(article_ids[2], None, true, 10)
            .expect("record smart feed open after unassign");
        assert_eq!(smart_feed_open.unread_zone_ids, [first_zone]);
        let source_open = db
            .record_article_open(article_ids[3], None, false, 10)
            .expect("record source open");
        assert_eq!(source_open.unread_zone_ids, [first_zone]);
        assert!(source_open.behavior_progress.is_empty());
        assert!(source_open.ready_behavior_zone_ids.is_empty());

        let conn = db.conn.lock().expect("lock database");
        let first_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_read_events WHERE zone_id = ?1",
                params![first_zone],
                |row| row.get(0),
            )
            .expect("count first-zone events");
        let second_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM zone_read_events WHERE zone_id = ?1",
                params![second_zone],
                |row| row.get(0),
            )
            .expect("count second-zone events");
        assert_eq!(first_count, 3);
        assert_eq!(second_count, 1);
    }

    #[test]
    fn behavior_learning_uses_recurring_batches_and_revision_safe_resets() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let feed_id = db
            .upsert_feed(
                &FeedMetadata {
                    url: "https://example.com/behavior-batches.xml".to_string(),
                    title: "Behavior Batches".to_string(),
                    description: None,
                    site_url: None,
                },
                Some("Technology"),
                None,
            )
            .expect("insert feed");
        let articles = (1..=30)
            .map(|index| NewArticle {
                title: format!("Subject{index} Perspective{index}"),
                content: format!("Behavior batch content {index}"),
                summary: Some(format!("Behavior batch summary {index}")),
                url: format!("https://example.com/behavior-batch-{index}"),
                author: None,
                published_at: Some("2026-08-21T00:00:00Z".to_string()),
            })
            .collect::<Vec<_>>();
        db.insert_articles(feed_id, &articles)
            .expect("insert articles");
        let article_ids = db
            .list_articles(feed_id)
            .expect("list articles")
            .into_iter()
            .map(|article| article.id)
            .collect::<Vec<_>>();
        assert_eq!(article_ids.len(), 30);
        let zone_id = db
            .create_zone("Learning", "technology analysis", "focused")
            .expect("create zone");
        db.assign_feed(zone_id, feed_id).expect("assign feed");
        for article_id in &article_ids {
            db.save_classification(zone_id, *article_id, "high")
                .expect("classify article");
        }

        for article_id in article_ids.iter().take(9) {
            db.record_article_open(*article_id, Some(zone_id), false, 10)
                .expect("record initial open");
        }
        assert!(db
            .ready_behavior_zone_ids(10)
            .expect("list ready zones")
            .is_empty());
        assert!(db
            .next_behavior_batch(zone_id, 10)
            .expect("load incomplete batch")
            .is_none());

        let tenth_open = db
            .record_article_open(article_ids[9], Some(zone_id), false, 10)
            .expect("record tenth open");
        assert_eq!(tenth_open.behavior_progress.len(), 1);
        assert_eq!(tenth_open.behavior_progress[0].zone_id, zone_id);
        assert_eq!(tenth_open.behavior_progress[0].pending_reads, 10);
        assert_eq!(tenth_open.ready_behavior_zone_ids, [zone_id]);
        let first_batch = db
            .next_behavior_batch(zone_id, 10)
            .expect("load first batch")
            .expect("first batch exists");
        db.fail_behavior_batch(zone_id, first_batch.revision, "Provider unavailable")
            .expect("record failure");
        assert_eq!(
            db.get_zone(zone_id)
                .expect("load failed zone")
                .expect("zone exists")
                .behavior_last_error
                .as_deref(),
            Some("Provider unavailable")
        );
        assert!(db
            .complete_behavior_batch(&first_batch, "Prefers practical product analysis.")
            .expect("complete first batch"));

        for article_id in article_ids.iter().skip(10).take(10) {
            db.record_article_open(*article_id, Some(zone_id), false, 10)
                .expect("record second-batch open");
        }
        let second_batch = db
            .next_behavior_batch(zone_id, 10)
            .expect("load second batch")
            .expect("second batch exists");
        assert_eq!(
            second_batch.current_profile,
            "Prefers practical product analysis."
        );
        assert!(db
            .complete_behavior_batch(&second_batch, "Prefers detailed product analysis.")
            .expect("complete second batch"));
        db.record_article_open(article_ids[19], Some(zone_id), false, 10)
            .expect("record duplicate open");
        let learned_zone = db
            .get_zone(zone_id)
            .expect("load learned zone")
            .expect("zone exists");
        assert_eq!(learned_zone.behavior_reads_processed, 20);
        assert_eq!(learned_zone.behavior_reads_pending, 0);

        for article_id in article_ids.iter().skip(20).take(10) {
            db.record_article_open(*article_id, Some(zone_id), false, 10)
                .expect("record stale-batch open");
        }
        let stale_batch = db
            .next_behavior_batch(zone_id, 10)
            .expect("load stale batch")
            .expect("stale batch exists");
        db.reset_zone_behavior(zone_id).expect("reset behavior");
        assert!(!db
            .complete_behavior_batch(&stale_batch, "Stale profile")
            .expect("reject stale batch"));

        db.record_article_open(article_ids[0], Some(zone_id), false, 10)
            .expect("record post-reset open");
        db.update_zone(zone_id, "Learning", "technology analysis", "focused")
            .expect("save unchanged brief");
        assert_eq!(
            db.get_zone(zone_id)
                .expect("load unchanged zone")
                .expect("zone exists")
                .behavior_reads_pending,
            1
        );
        db.update_zone(zone_id, "Learning", "science analysis", "focused")
            .expect("change brief");
        let reset_zone = db
            .get_zone(zone_id)
            .expect("load reset zone")
            .expect("zone exists");
        assert!(reset_zone.behavior_profile.is_empty());
        assert_eq!(reset_zone.behavior_reads_processed, 0);
        assert_eq!(reset_zone.behavior_reads_pending, 0);
        assert_eq!(reset_zone.behavior_last_error, None);
    }

    #[test]
    fn provider_settings_are_saved_in_one_transaction() {
        let db = Db::new(Path::new(":memory:")).expect("create in-memory db");
        let settings = vec![
            Setting {
                key: "ai_provider".to_string(),
                value: "openai".to_string(),
            },
            Setting {
                key: "ai.openai.model".to_string(),
                value: "gpt-4o-mini".to_string(),
            },
            Setting {
                key: "ai.openai.api_key".to_string(),
                value: "test-key".to_string(),
            },
        ];

        db.update_settings(&settings)
            .expect("save provider settings transaction");

        let saved: HashMap<_, _> = db
            .get_settings()
            .expect("load provider settings")
            .into_iter()
            .map(|setting| (setting.key, setting.value))
            .collect();
        assert_eq!(saved.get("ai_provider").map(String::as_str), Some("openai"));
        assert_eq!(
            saved.get("ai.openai.model").map(String::as_str),
            Some("gpt-4o-mini")
        );
        assert_eq!(
            saved.get("ai.openai.api_key").map(String::as_str),
            Some("test-key")
        );
    }
}
