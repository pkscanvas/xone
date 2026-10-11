#[allow(dead_code)] // dedup/origin fields document and test the generated catalog audit.
pub struct DefaultFeed {
    pub title: &'static str,
    pub url: &'static str,
    pub category: &'static str,
    pub region: Option<&'static str>,
    pub dedup_key: &'static str,
    pub origin: &'static str,
}

include!("default_catalog.rs");
include!("current_app_catalog.rs");
include!("worldmonitor_catalog.rs");
include!("native_feed_catalog.rs");
include!("default_feed_merges.rs");

include!("starter_zones.rs");

/// True when a Google News `site:` proxy in the installed-app snapshot has been
/// superseded by the publisher's own feed in `NATIVE_FEED_RECORDS`.
pub fn is_replaced_by_native_feed(url: &str) -> bool {
    NATIVE_FEED_REPLACEMENTS
        .iter()
        .any(|&(retired, _)| retired == url)
}

pub fn default_feeds() -> impl Iterator<Item = &'static DefaultFeed> {
    CURRENT_APP_FEED_RECORDS
        .iter()
        .filter(|feed| !is_replaced_by_native_feed(feed.url))
        .chain(WORLDMONITOR_FEED_RECORDS.iter())
        .chain(NATIVE_FEED_RECORDS.iter())
}

pub fn retired_topic_query_feeds() -> impl Iterator<Item = &'static DefaultFeed> {
    AUDITED_FEED_RECORDS
        .iter()
        .filter(|feed| feed.dedup_key.starts_with("topic-query:"))
        .filter(|feed| {
            !CURRENT_APP_FEED_RECORDS
                .iter()
                .any(|current| current.url == feed.url)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn catalog_matches_installed_app_source_snapshot() {
        let feeds = CURRENT_APP_FEED_RECORDS.iter().collect::<Vec<_>>();
        let urls = feeds.iter().map(|feed| feed.url).collect::<HashSet<_>>();
        let dedup_keys = feeds
            .iter()
            .map(|feed| feed.dedup_key)
            .collect::<HashSet<_>>();

        assert_eq!(feeds.len(), 460);
        assert_eq!(urls.len(), feeds.len());
        assert_eq!(dedup_keys.len(), feeds.len());
        assert_eq!(
            feeds
                .iter()
                .filter(|feed| feed.origin == "installed-app")
                .count(),
            feeds.len()
        );
        assert_eq!(retired_topic_query_feeds().count(), 0);
        assert_eq!(
            feeds
                .iter()
                .filter(|feed| feed.url.starts_with("https://news.google.com/rss/search?"))
                .count(),
            244
        );
        assert_eq!(
            feeds
                .iter()
                .filter(|feed| feed.url.starts_with("https://www.livemint.com/rss/"))
                .count(),
            16
        );
        let mint_ai = feeds
            .iter()
            .find(|feed| feed.url == "https://www.livemint.com/rss/AI")
            .expect("LiveMint AI feed");
        assert_eq!(mint_ai.category, "AI");
        assert_eq!(mint_ai.region, None);
        let mint_news = feeds
            .iter()
            .find(|feed| feed.url == "https://www.livemint.com/rss/news")
            .expect("LiveMint news feed");
        assert_eq!(mint_news.category, "General News");
        assert_eq!(mint_news.region, Some("Asia"));
        assert!(feeds.iter().all(|feed| {
            !feed.title.is_empty()
                && !feed.url.is_empty()
                && !feed.category.is_empty()
                && !feed.dedup_key.is_empty()
                && (feed.region.is_none() || feed.category == "General News")
        }));

        let verge = feeds
            .iter()
            .find(|feed| {
                feed.url == "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml"
            })
            .expect("The Verge AI feed");
        assert_eq!(verge.title, "The Verge");
        assert_eq!(verge.category, "AI");
        assert_eq!(verge.region, None);

        let readless_ai_feeds = [
            ("OpenAI News", "https://openai.com/news/rss.xml"),
            ("Hugging Face Blog", "https://huggingface.co/blog/feed.xml"),
            (
                "MIT Technology Review AI",
                "https://www.technologyreview.com/topic/artificial-intelligence/feed/",
            ),
            ("Google AI Blog", "https://blog.google/technology/ai/rss/"),
            ("MarkTechPost", "https://www.marktechpost.com/feed/"),
            ("arXiv cs.AI", "https://rss.arxiv.org/rss/cs.AI"),
            ("Ahead of AI", "https://magazine.sebastianraschka.com/feed"),
            ("The Gradient", "https://thegradient.pub/rss/"),
            ("Last Week in AI", "https://lastweekin.ai/feed"),
            (
                "The Verge",
                "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml",
            ),
        ];
        for (title, url) in readless_ai_feeds {
            let feed = feeds
                .iter()
                .find(|feed| feed.url == url)
                .unwrap_or_else(|| panic!("missing Readless AI feed: {title}"));
            assert_eq!(feed.title, title);
            assert_eq!(feed.category, "AI");
            assert_eq!(feed.region, None);
        }

        for &(retired, retained) in RETIRED_DEFAULT_FEED_MERGES {
            assert_ne!(retired, retained);
            assert!(urls.contains(retained));
            assert!(!urls.contains(retired));
        }
    }

    #[test]
    fn native_feeds_replace_their_google_news_proxies() {
        let snapshot_urls = CURRENT_APP_FEED_RECORDS
            .iter()
            .map(|feed| feed.url)
            .collect::<HashSet<_>>();
        let snapshot_by_url = CURRENT_APP_FEED_RECORDS
            .iter()
            .map(|feed| (feed.url, feed))
            .collect::<std::collections::HashMap<_, _>>();
        let native_by_url = NATIVE_FEED_RECORDS
            .iter()
            .map(|feed| (feed.url, feed))
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(NATIVE_FEED_REPLACEMENTS.len(), 26);
        assert_eq!(NATIVE_FEED_RECORDS.len(), NATIVE_FEED_REPLACEMENTS.len());

        for &(proxy, native) in NATIVE_FEED_REPLACEMENTS {
            assert_ne!(proxy, native);
            // The proxy stays in the frozen snapshot and the native feed is new.
            assert!(
                snapshot_urls.contains(proxy),
                "proxy not in snapshot: {proxy}"
            );
            assert!(
                !snapshot_urls.contains(native),
                "native already seeded: {native}"
            );
            assert!(
                proxy.starts_with("https://news.google.com/rss/search?"),
                "replacement target is not a Google News proxy: {proxy}"
            );
            assert!(
                !native.starts_with("https://news.google.com/rss/search?"),
                "replacement is still a Google News proxy: {native}"
            );

            // Display name, category, and region are user-facing and carry over intact.
            let old = snapshot_by_url[proxy];
            let new = native_by_url
                .get(native)
                .unwrap_or_else(|| panic!("no record for native feed: {native}"));
            assert_eq!(old.title, new.title);
            assert_eq!(old.category, new.category);
            assert_eq!(old.region, new.region);
            assert_eq!(new.origin, "native-feed");
            assert!(new.dedup_key.starts_with("native-feed:"));
        }

        // The seeded catalog serves the native feed and never the proxy it replaced.
        let seeded = default_feeds().map(|feed| feed.url).collect::<HashSet<_>>();
        for &(proxy, native) in NATIVE_FEED_REPLACEMENTS {
            assert!(
                !seeded.contains(proxy),
                "retired proxy still seeded: {proxy}"
            );
            assert!(seeded.contains(native), "native feed not seeded: {native}");
            assert!(is_replaced_by_native_feed(proxy));
            assert!(!is_replaced_by_native_feed(native));
        }

        let seeded_all = default_feeds().collect::<Vec<_>>();
        assert_eq!(
            seeded_all.len(),
            CURRENT_APP_FEED_RECORDS.len() + WORLDMONITOR_FEED_RECORDS.len()
        );
        assert_eq!(seeded.len(), seeded_all.len());
        assert_eq!(
            seeded_all
                .iter()
                .map(|feed| feed.dedup_key)
                .collect::<HashSet<_>>()
                .len(),
            seeded_all.len()
        );
    }

    #[test]
    fn worldmonitor_records_extend_the_catalog_without_colliding() {
        let snapshot = CURRENT_APP_FEED_RECORDS.iter().collect::<Vec<_>>();
        let adopted = WORLDMONITOR_FEED_RECORDS.iter().collect::<Vec<_>>();

        assert_eq!(adopted.len(), 34);
        assert!(adopted.iter().all(|feed| feed.origin == "worldmonitor"));
        assert!(adopted
            .iter()
            .all(|feed| feed.dedup_key.starts_with("worldmonitor:")));

        // Adopted sources are native publisher feeds, never Google News proxies, so
        // the four-day publisher-window cap has nothing to merge or rewrite.
        assert!(adopted
            .iter()
            .all(|feed| !feed.url.starts_with("https://news.google.com/rss/search?")));

        // General News keeps a region; every other category never carries one.
        assert!(adopted.iter().all(|feed| {
            !feed.title.is_empty()
                && !feed.url.is_empty()
                && !feed.category.is_empty()
                && (feed.region.is_some() == (feed.category == "General News"))
        }));

        // Every adopted category already exists in the installed-app snapshot, so the
        // Sources page and Xone picker gain rows, not new groups.
        let snapshot_categories = snapshot
            .iter()
            .map(|feed| feed.category)
            .collect::<HashSet<_>>();
        let snapshot_regions = snapshot
            .iter()
            .filter_map(|feed| feed.region)
            .collect::<HashSet<_>>();
        for feed in &adopted {
            assert!(
                snapshot_categories.contains(feed.category),
                "unknown category: {}",
                feed.category
            );
            if let Some(region) = feed.region {
                assert!(
                    snapshot_regions.contains(region),
                    "unknown region: {region}"
                );
            }
        }

        let combined = default_feeds().collect::<Vec<_>>();
        assert_eq!(combined.len(), snapshot.len() + adopted.len());
        assert_eq!(
            combined
                .iter()
                .map(|feed| feed.url)
                .collect::<HashSet<_>>()
                .len(),
            combined.len()
        );
        assert_eq!(
            combined
                .iter()
                .map(|feed| feed.dedup_key)
                .collect::<HashSet<_>>()
                .len(),
            combined.len()
        );
        for &(retired, _) in RETIRED_DEFAULT_FEED_MERGES {
            assert!(combined.iter().all(|feed| feed.url != retired));
        }
    }
}
