// Native publisher feeds that replace a Google News `site:` proxy for the same
// publisher. Each native URL was fetched live and confirmed to return a parseable
// feed with recent items; stale, paywalled, blocked, and HTML-only endpoints were
// rejected. Titles, categories, and regions are carried over unchanged from the
// installed-app snapshot, which is never edited: `default_feeds` filters the retired
// proxy out and seeding merges it into the native row, preserving stored articles and
// Xone assignments.
#[rustfmt::skip]
pub const NATIVE_FEED_RECORDS: &[DefaultFeed] = &[
    DefaultFeed { title: "a16z Blog", url: "https://www.a16z.news/feed", category: "Startups & VC", region: None, dedup_key: "native-feed:bbcd525524d851e4", origin: "native-feed" },
    DefaultFeed { title: "Arabian Business", url: "https://arabianbusiness.com/feed", category: "Gulf Business", region: None, dedup_key: "native-feed:333c2045deac0137", origin: "native-feed" },
    DefaultFeed { title: "Arab News", url: "https://arabnews.com/rss", category: "General News", region: Some("Middle East"), dedup_key: "native-feed:7f1e21ea4ae58387", origin: "native-feed" },
    DefaultFeed { title: "Arms Control Association", url: "https://armscontrol.org/rss.xml", category: "Defense/Geopolitics", region: None, dedup_key: "native-feed:7c75d5d880790a62", origin: "native-feed" },
    DefaultFeed { title: "Bellingcat", url: "https://bellingcat.com/feed", category: "Defense/Geopolitics", region: None, dedup_key: "native-feed:853f5f4577837468", origin: "native-feed" },
    DefaultFeed { title: "Bloomberg Markets", url: "https://feeds.bloomberg.com/markets/news.rss", category: "Markets", region: None, dedup_key: "native-feed:03c4eab0876a67e4", origin: "native-feed" },
    DefaultFeed { title: "Pentagon", url: "https://www.war.gov/DesktopModules/ArticleCS/RSS.ashx?ContentType=1&Site=945", category: "Government", region: None, dedup_key: "native-feed:3edc77e36af93787", origin: "native-feed" },
    DefaultFeed { title: "ECFR", url: "https://ecfr.eu/rss", category: "Policy & Think Tanks", region: None, dedup_key: "native-feed:009d5750fe497a99", origin: "native-feed" },
    DefaultFeed { title: "EFF News", url: "https://eff.org/rss.xml", category: "Technology Policy", region: None, dedup_key: "native-feed:3140bfa0f142f7e8", origin: "native-feed" },
    DefaultFeed { title: "EU Startups", url: "https://eu-startups.com/rss", category: "Startups & VC", region: None, dedup_key: "native-feed:03eeee0ecbc15a3c", origin: "native-feed" },
    DefaultFeed { title: "Euractiv Digital", url: "https://euractiv.com/rss", category: "Technology Policy", region: None, dedup_key: "native-feed:708d576f9d015b54", origin: "native-feed" },
    DefaultFeed { title: "FX Empire Gold", url: "https://www.fxempire.com/api/v1/en/articles/rss/news", category: "Precious Metals", region: None, dedup_key: "native-feed:aee186df55cf8375", origin: "native-feed" },
    DefaultFeed { title: "German Marshall Fund", url: "https://gmfus.org/rss.xml", category: "Policy & Think Tanks", region: None, dedup_key: "native-feed:5732c441d937bfc2", origin: "native-feed" },
    DefaultFeed { title: "Investing.com News", url: "https://www.investing.com/rss/news.rss", category: "Markets", region: None, dedup_key: "native-feed:1c3a146315ddebcf", origin: "native-feed" },
    DefaultFeed { title: "Iran International", url: "https://iranintl.com/feed", category: "General News", region: Some("Middle East"), dedup_key: "native-feed:23fcd931604d0169", origin: "native-feed" },
    DefaultFeed { title: "DOJ", url: "https://www.justice.gov/news/rss?type=press_release", category: "Government", region: None, dedup_key: "native-feed:db8a8597f1ed83d3", origin: "native-feed" },
    DefaultFeed { title: "Kyiv Independent", url: "https://kyivindependent.com/news-archive/rss/", category: "General News", region: Some("Europe"), dedup_key: "native-feed:11137c56a4f13f75", origin: "native-feed" },
    DefaultFeed { title: "LAVCA (LATAM)", url: "https://lavca.org/rss", category: "Startups & VC", region: None, dedup_key: "native-feed:ede35487f99c1dd3", origin: "native-feed" },
    DefaultFeed { title: "Lowy Institute", url: "https://lowyinstitute.org/rss", category: "Policy & Think Tanks", region: None, dedup_key: "native-feed:5be3d9a1f8d044f8", origin: "native-feed" },
    DefaultFeed { title: "MarketWatch", url: "https://feeds.content.dowjones.io/public/rss/mw_topstories", category: "Finance", region: None, dedup_key: "native-feed:df1d388d5efdef40", origin: "native-feed" },
    DefaultFeed { title: "RUSI", url: "https://rusi.org/rss/whats-new.xml", category: "Policy & Think Tanks", region: None, dedup_key: "native-feed:817618082489abd9", origin: "native-feed" },
    DefaultFeed { title: "Sequoia Blog", url: "https://sequoiacap.com/feed", category: "Startups & VC", region: None, dedup_key: "native-feed:0deace4b66cbb7c8", origin: "native-feed" },
    DefaultFeed { title: "State Dept", url: "https://www.state.gov/rss-feed/press-releases/feed/", category: "Government", region: None, dedup_key: "native-feed:28fe2077f87d66ee", origin: "native-feed" },
    DefaultFeed { title: "Tech in Asia", url: "https://techinasia.com/feed", category: "Startups & VC", region: None, dedup_key: "native-feed:af01979e902f385b", origin: "native-feed" },
    DefaultFeed { title: "The Block", url: "https://theblock.co/feed", category: "Crypto", region: None, dedup_key: "native-feed:871cb9f1a6e66d6f", origin: "native-feed" },
    DefaultFeed { title: "The Next Web", url: "https://thenextweb.com/rss", category: "Startups & VC", region: None, dedup_key: "native-feed:1c75757ca26c0eb8", origin: "native-feed" },
];

/// `(retired Google News proxy, native publisher feed)`.
pub const NATIVE_FEED_REPLACEMENTS: &[(&str, &str)] = &[
    (
        "https://news.google.com/rss/search?q=site:a16z.com+OR+\"Andreessen+Horowitz\"+blog+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://www.a16z.news/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:arabianbusiness.com+(Saudi+Arabia+OR+UAE+OR+GCC)+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://arabianbusiness.com/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:arabnews.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://arabnews.com/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:armscontrol.org+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://armscontrol.org/rss.xml",
    ),
    (
        "https://news.google.com/rss/search?q=site:bellingcat.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://bellingcat.com/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:bloomberg.com+markets+when:1d&hl=en-US&gl=US&ceid=US:en",
        "https://feeds.bloomberg.com/markets/news.rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:defense.gov+OR+Pentagon+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://www.war.gov/DesktopModules/ArticleCS/RSS.ashx?ContentType=1&Site=945",
    ),
    (
        "https://news.google.com/rss/search?q=site:ecfr.eu+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://ecfr.eu/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:eff.org+OR+\"Electronic+Frontier+Foundation\"+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://eff.org/rss.xml",
    ),
    (
        "https://news.google.com/rss/search?q=site:eu-startups.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://eu-startups.com/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:euractiv.com+digital+OR+tech+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://euractiv.com/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:fxempire.com+gold+when:1d&hl=en-US&gl=US&ceid=US:en",
        "https://www.fxempire.com/api/v1/en/articles/rss/news",
    ),
    (
        "https://news.google.com/rss/search?q=site:gmfus.org+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://gmfus.org/rss.xml",
    ),
    (
        "https://news.google.com/rss/search?q=site:investing.com+markets+when:1d&hl=en-US&gl=US&ceid=US:en",
        "https://www.investing.com/rss/news.rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:iranintl.com+when:2d&hl=en-US&gl=US&ceid=US:en",
        "https://iranintl.com/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:justice.gov+OR+\"Justice+Department\"+DOJ+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://www.justice.gov/news/rss?type=press_release",
    ),
    (
        "https://news.google.com/rss/search?q=site:kyivindependent.com+when:3d&hl=en-US&gl=US&ceid=US:en",
        "https://kyivindependent.com/news-archive/rss/",
    ),
    (
        "https://news.google.com/rss/search?q=site:lavca.org+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://lavca.org/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:lowyinstitute.org+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://lowyinstitute.org/rss",
    ),
    (
        "https://news.google.com/rss/search?q=site:marketwatch.com+markets+when:1d&hl=en-US&gl=US&ceid=US:en",
        "https://feeds.content.dowjones.io/public/rss/mw_topstories",
    ),
    (
        "https://news.google.com/rss/search?q=site:rusi.org+when:3d&hl=en-US&gl=US&ceid=US:en",
        "https://rusi.org/rss/whats-new.xml",
    ),
    (
        "https://news.google.com/rss/search?q=site:sequoiacap.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://sequoiacap.com/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:state.gov+OR+\"State+Department\"+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://www.state.gov/rss-feed/press-releases/feed/",
    ),
    (
        "https://news.google.com/rss/search?q=site:techinasia.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://techinasia.com/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:theblock.co+when:1d&hl=en-US&gl=US&ceid=US:en",
        "https://theblock.co/feed",
    ),
    (
        "https://news.google.com/rss/search?q=site:thenextweb.com+when:4d&hl=en-US&gl=US&ceid=US:en",
        "https://thenextweb.com/rss",
    ),
];
