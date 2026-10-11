use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::DateTime;

use crate::content::clean_text;

const MATCH_WINDOW_SECONDS: i64 = 72 * 60 * 60;

/// Upper bound on how many stored reports a single lookup scores. Candidates
/// are drawn from the rarest tokens first, so the cap only ever discards
/// entries that share nothing but common words with the incoming report.
const MAX_SCORED_CANDIDATES: usize = 512;

/// How much history the in-memory index keeps. Matching only ever compares
/// reports within `MATCH_WINDOW_SECONDS`, so anything older can never produce
/// a hit and is dropped instead of growing the index for the whole retention
/// period.
pub const INDEX_RETENTION_SECONDS: i64 = 96 * 60 * 60;

#[derive(Debug, Clone)]
pub struct IndexedArticle {
    pub article_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub event_at: String,
}

#[derive(Default)]
pub struct ArticleIndex {
    entries: Vec<ArticleEntry>,
    entries_by_token: HashMap<String, Vec<usize>>,
}

struct ArticleEntry {
    article_id: i64,
    fingerprint: ArticleFingerprint,
    event_timestamp: Option<i64>,
}

struct ArticleFingerprint {
    title_tokens: HashSet<String>,
    all_tokens: HashSet<String>,
    action_tokens: HashSet<String>,
}

impl ArticleIndex {
    pub fn from_articles(articles: Vec<IndexedArticle>) -> Self {
        let mut index = Self::default();
        for article in articles {
            index.insert(
                article.article_id,
                &article.title,
                article.summary.as_deref(),
                &article.event_at,
            );
        }
        index
    }

    pub fn find_duplicate(
        &self,
        title: &str,
        summary: Option<&str>,
        event_at: &str,
    ) -> Option<i64> {
        let fingerprint = ArticleFingerprint::new(title, summary);
        if fingerprint.title_tokens.is_empty() {
            return None;
        }

        let event_timestamp = parse_timestamp(event_at);
        self.candidate_entries(&fingerprint)
            .into_iter()
            .filter_map(|index| {
                let entry = &self.entries[index];
                if !within_match_window(event_timestamp, entry.event_timestamp) {
                    return None;
                }
                match_score(self, &fingerprint, &entry.fingerprint)
                    .map(|score| (entry.article_id, score, index))
            })
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| right.2.cmp(&left.2))
            })
            .map(|(article_id, _, _)| article_id)
    }

    /// Collects the entries worth scoring, starting from the rarest shared
    /// token. Common words like `google` or `report` sit in huge posting lists
    /// that almost never decide a match, so walking rarest-first keeps a lookup
    /// proportional to the distinctive vocabulary of the incoming report rather
    /// than to the size of the index.
    fn candidate_entries(&self, fingerprint: &ArticleFingerprint) -> HashSet<usize> {
        let mut postings = fingerprint
            .all_tokens
            .iter()
            .filter_map(|token| {
                self.entries_by_token
                    .get(token)
                    .map(|entries| (entries.len(), token, entries))
            })
            .collect::<Vec<_>>();
        // Ties break on the token itself so candidate selection stays
        // deterministic regardless of hash iteration order.
        postings.sort_unstable_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(right.1)));

        let mut candidates = HashSet::new();
        for (_, _, entries) in postings {
            if candidates.len() >= MAX_SCORED_CANDIDATES {
                break;
            }
            candidates.extend(entries.iter().copied());
        }
        candidates
    }

    /// Tokens this rare carry real evidence; anything more common is treated as
    /// background vocabulary.
    fn is_distinctive(&self, token: &str) -> bool {
        self.document_frequency(token) <= (self.entries.len() / 50).max(12)
    }

    fn document_frequency(&self, token: &str) -> usize {
        self.entries_by_token
            .get(token)
            .map_or(usize::MAX, |entries| entries.len())
    }

    pub fn insert(&mut self, article_id: i64, title: &str, summary: Option<&str>, event_at: &str) {
        let fingerprint = ArticleFingerprint::new(title, summary);
        let index = self.entries.len();
        for token in &fingerprint.all_tokens {
            self.entries_by_token
                .entry(token.clone())
                .or_default()
                .push(index);
        }
        self.entries.push(ArticleEntry {
            article_id,
            fingerprint,
            event_timestamp: parse_timestamp(event_at),
        });
    }

    /// Drops entries that can no longer match anything, keeping the index and
    /// its token weights proportional to the live matching window rather than
    /// to the full article retention period.
    pub fn prune_before(&mut self, cutoff_timestamp: i64) {
        let expired = |entry: &ArticleEntry| {
            entry
                .event_timestamp
                .is_none_or(|timestamp| timestamp < cutoff_timestamp)
        };
        if !self.entries.iter().any(expired) {
            return;
        }

        let retained = std::mem::take(&mut self.entries)
            .into_iter()
            .filter(|entry| !expired(entry))
            .collect::<Vec<_>>();
        self.entries_by_token.clear();
        for (index, entry) in retained.iter().enumerate() {
            for token in &entry.fingerprint.all_tokens {
                self.entries_by_token
                    .entry(token.clone())
                    .or_default()
                    .push(index);
            }
        }
        self.entries = retained;
    }

    /// Marks the current end of the index so speculative inserts can be undone.
    pub fn checkpoint(&self) -> usize {
        self.entries.len()
    }

    /// Drops every entry added since `checkpoint`. Inserts only ever append, so
    /// truncating the entry list and its posting lists restores the exact prior
    /// state — used when a database transaction fails after its articles were
    /// already indexed.
    pub fn rollback_to(&mut self, checkpoint: usize) {
        if checkpoint >= self.entries.len() {
            return;
        }
        self.entries.truncate(checkpoint);
        self.entries_by_token.retain(|_, entries| {
            entries.retain(|index| *index < checkpoint);
            !entries.is_empty()
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl ArticleFingerprint {
    fn new(title: &str, summary: Option<&str>) -> Self {
        let title_tokens = matching_tokens(strip_source_attribution(title));
        let summary_tokens = usable_summary(summary)
            .map(|summary| matching_tokens(&summary))
            .unwrap_or_default();
        let all_tokens: HashSet<String> = title_tokens.union(&summary_tokens).cloned().collect();
        let action_tokens = title_tokens
            .iter()
            .filter(|token| is_action_token(token))
            .cloned()
            .collect();
        Self {
            title_tokens,
            all_tokens,
            action_tokens,
        }
    }
}

pub fn normalized_title(title: &str) -> String {
    title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Removes the trailing publisher credit that Google News and many syndicated
/// feeds append to headlines (`Headline - The Verge`). The credit is pure noise
/// for matching: the same story reaches us with a different suffix from every
/// source, which both dilutes the shared-token ratio and can make two unrelated
/// stories look alike when they happen to share a publisher.
fn strip_source_attribution(title: &str) -> &str {
    const SEPARATORS: [&str; 4] = [" - ", " | ", " — ", " – "];
    let mut head = title.trim();

    for separator in SEPARATORS {
        let Some(index) = head.rfind(separator) else {
            continue;
        };
        let (candidate_head, tail) = (&head[..index], &head[index + separator.len()..]);
        let tail_words = tail.split_whitespace().collect::<Vec<_>>();
        // A publisher credit is short and title-cased; anything else is part of
        // the headline and must be kept.
        let looks_like_credit = (1..=4).contains(&tail_words.len())
            && tail_words.iter().all(|word| {
                matches!(word.to_lowercase().as_str(), "of" | "the" | "and" | "for")
                    || word
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_uppercase() || ch.is_ascii_digit())
            });
        if looks_like_credit && candidate_head.split_whitespace().count() >= 4 {
            head = candidate_head.trim_end();
        }
    }

    head
}

fn match_score(
    index: &ArticleIndex,
    candidate: &ArticleFingerprint,
    existing: &ArticleFingerprint,
) -> Option<f64> {
    let shared_title = candidate
        .title_tokens
        .intersection(&existing.title_tokens)
        .count();
    let title_score = containment_similarity(&candidate.title_tokens, &existing.title_tokens);
    // Near-identical headlines are accepted outright, but the overlap has to
    // say something about the story. Recurring commercial formats ("Hoka Coupon
    // Codes: 30% Off in September 2026") reach this threshold on their template
    // and the date alone, while the brand that identifies them is the one word
    // they do not share.
    if shared_title >= 3
        && title_score >= 0.80
        && candidate
            .title_tokens
            .intersection(&existing.title_tokens)
            .any(|token| index.is_distinctive(token) && is_substantive_token(token))
    {
        return Some(1.0 + title_score);
    }

    let shared_actions = candidate
        .action_tokens
        .intersection(&existing.action_tokens)
        .count();
    if shared_actions == 0 || shared_title < 2 || title_score < 0.45 {
        return None;
    }

    let distinctive_shared = candidate
        .all_tokens
        .intersection(&existing.all_tokens)
        .filter(|token| !is_action_token(token) && index.is_distinctive(token))
        .count();
    let shared_all = candidate
        .all_tokens
        .intersection(&existing.all_tokens)
        .count();
    if distinctive_shared < 2 || shared_all < 4 {
        return None;
    }

    let combined_score = weighted_containment(index, &candidate.all_tokens, &existing.all_tokens);
    (combined_score >= if title_score >= 0.70 { 0.20 } else { 0.30 })
        .then_some(title_score + combined_score)
}

fn containment_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(right).count() as f64 / left.len().min(right.len()) as f64
}

fn weighted_containment(
    index: &ArticleIndex,
    left: &HashSet<String>,
    right: &HashSet<String>,
) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let total_documents = index.entries.len().max(1) as f64;
    let weight = |token: &String| {
        let document_frequency = index
            .entries_by_token
            .get(token)
            .map_or(1, |entries| entries.len()) as f64;
        let rarity = (total_documents / document_frequency).ln_1p();
        let signal_weight = if is_action_token(token) || is_model_or_number_token(token) {
            1.5
        } else {
            1.0
        };
        (1.0 + rarity) * signal_weight
    };
    let overlap: f64 = left.intersection(right).map(&weight).sum();
    let left_total: f64 = left.iter().map(&weight).sum();
    let right_total: f64 = right.iter().map(&weight).sum();
    overlap / left_total.min(right_total)
}

fn matching_tokens(input: &str) -> HashSet<String> {
    let cleaned = clean_text(input);
    let mut tokens = HashSet::new();

    for raw in cleaned.split_whitespace() {
        let compact = raw
            .chars()
            .filter(|ch| ch.is_alphanumeric())
            .collect::<String>();
        if raw.chars().all(|ch| ch.is_alphanumeric())
            || compact.chars().any(|ch| ch.is_ascii_digit())
            || compact.eq_ignore_ascii_case("zai")
        {
            insert_token(&mut tokens, &compact);
        }

        for part in raw.split(|ch: char| !ch.is_alphanumeric()) {
            insert_token(&mut tokens, part);
        }
    }

    tokens
}

fn insert_token(tokens: &mut HashSet<String>, raw: &str) {
    if raw.is_empty() {
        return;
    }
    let lowercase = raw.to_lowercase();
    let token = canonical_token(&lowercase);
    if is_stopword(&token)
        || !(token.chars().count() >= 4
            || is_short_ai_term(&token)
            || (token.chars().any(|ch| ch.is_ascii_digit()) && token.chars().count() >= 2))
    {
        return;
    }
    tokens.insert(token.into_owned());
}

fn canonical_token(token: &str) -> Cow<'_, str> {
    if let Some(canonical) = canonical_action(token) {
        return Cow::Borrowed(canonical);
    }
    let singular = singular_form(token);
    match canonical_action(&singular) {
        Some(canonical) => Cow::Borrowed(canonical),
        None => singular,
    }
}

/// Collapses regular plurals so `racks`/`rack` and `companies`/`company` share
/// a token. Deliberately conservative: it never touches short words and leaves
/// `-ss`/`-us`/`-is` endings alone, because over-stemming invents collisions
/// between unrelated words while under-stemming only costs a little evidence.
fn singular_form(token: &str) -> Cow<'_, str> {
    if token.len() < 5 || !token.is_ascii() {
        return Cow::Borrowed(token);
    }
    let stem = |suffix: usize| &token[..token.len() - suffix];
    if token.ends_with("ies") {
        return Cow::Owned(format!("{}y", stem(3)));
    }
    // Only `-es` endings that a singular could not have produced: `taxes`,
    // `watches`, `classes`. Everything else (`prices`, `devices`) drops the
    // trailing `s` below so it lines up with its own singular.
    if token.ends_with("es") && (stem(2).ends_with(['x', 'z', 'h', 'o']) || stem(2).ends_with("ss"))
    {
        return Cow::Borrowed(stem(2));
    }
    if token.ends_with('s')
        && !token.ends_with("ss")
        && !token.ends_with("us")
        && !token.ends_with("is")
    {
        return Cow::Borrowed(stem(1));
    }
    Cow::Borrowed(token)
}

fn canonical_action(token: &str) -> Option<&'static str> {
    Some(match token {
        "acquisition" | "acquisitions" | "acquire" | "acquires" | "acquired" | "acquiring"
        | "buy" | "buys" | "bought" | "buying" => "acquire",
        "hack" | "hacks" | "hacked" | "hacking" | "breach" | "breaches" | "breached"
        | "breaching" => "hack",
        "transcribe" | "transcribes" | "transcribed" | "transcribing" | "transcription"
        | "transcriptions" => "transcribe",
        "release" | "releases" | "released" | "releasing" | "launch" | "launches" | "launched"
        | "launching" | "debut" | "debuts" | "debuted" | "ships" | "shipped" | "shipping"
        | "unveil" | "unveils" | "unveiled" | "introduce" | "introduces" | "introduced"
        | "announce" | "announces" | "announced" | "announcement" | "reveal" | "reveals"
        | "revealed" => "release",
        "raise" | "raises" | "raised" | "raising" | "funding" | "invest" | "invests"
        | "invested" | "investment" | "investments" => "fund",
        "ban" | "bans" | "banned" | "banning" | "block" | "blocks" | "blocked" | "blocking" => {
            "ban"
        }
        "partner" | "partners" | "partnered" | "partnership" | "partnerships" => "partner",
        "layoff" | "layoffs" | "fire" | "fires" | "fired" | "firing" => "layoff",
        "resign" | "resigns" | "resigned" | "resignation" | "quit" | "quits" | "stepped" => {
            "resign"
        }
        "hire" | "hires" | "hired" | "hiring" | "appoint" | "appoints" | "appointed" => "hire",
        "approve" | "approves" | "approved" | "approving" | "approval" => "approve",
        "sue" | "sues" | "sued" | "suing" | "lawsuit" => "sue",
        "deal" | "deals" | "agreement" | "agreements" | "pay" | "pays" | "paid" | "paying"
        | "rent" | "rents" | "rented" | "renting" | "settle" | "settles" | "settled"
        | "settlement" | "settlements" => "deal",
        "deploy" | "deploys" | "deployed" | "deploying" | "deployment" | "deployments"
        | "production" | "operational" => "deploy",
        "find" | "finds" | "found" | "discover" | "discovers" | "discovered" => "find",
        "dies" | "died" | "dead" => "death",
        _ => return None,
    })
}

fn is_action_token(token: &str) -> bool {
    matches!(
        token,
        "acquire"
            | "hack"
            | "transcribe"
            | "release"
            | "fund"
            | "ban"
            | "partner"
            | "layoff"
            | "resign"
            | "hire"
            | "approve"
            | "sue"
            | "deal"
            | "deploy"
            | "find"
            | "death"
    )
}

/// Tokens that identify a story rather than the recurring shell around it.
/// Dates and promotional vocabulary repeat across unrelated affiliate posts, so
/// on their own they are not evidence that two headlines describe one story.
fn is_substantive_token(token: &str) -> bool {
    !matches!(
        token,
        "coupon"
            | "promo"
            | "promotion"
            | "discount"
            | "voucher"
            | "code"
            | "sale"
            | "saving"
            | "january"
            | "february"
            | "march"
            | "april"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    ) && !is_year_token(token)
}

fn is_year_token(token: &str) -> bool {
    token.len() == 4
        && token.chars().all(|ch| ch.is_ascii_digit())
        && matches!(token.parse::<u32>(), Ok(1900..=2100))
}

fn is_model_or_number_token(token: &str) -> bool {
    token.chars().any(|ch| ch.is_ascii_digit()) || is_short_ai_term(token)
}

fn is_short_ai_term(token: &str) -> bool {
    matches!(token, "ai" | "agi" | "gpt" | "glm" | "llm" | "zai")
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "again"
            | "against"
            | "also"
            | "been"
            | "before"
            | "being"
            | "between"
            | "could"
            | "does"
            | "from"
            | "have"
            | "into"
            | "just"
            | "more"
            | "most"
            | "much"
            | "only"
            | "other"
            | "over"
            | "says"
            | "some"
            | "still"
            | "such"
            | "than"
            | "that"
            | "their"
            | "them"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "through"
            | "under"
            | "very"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "with"
            | "would"
            | "your"
    )
}

fn usable_summary(summary: Option<&str>) -> Option<String> {
    let cleaned = clean_text(summary?);
    let before_comments = find_ascii_case_insensitive(&cleaned, "comments url:")
        .map_or(cleaned.as_str(), |index| &cleaned[..index]);
    let trimmed = before_comments.trim();
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || starts_with_ascii_case_insensitive(trimmed, "article url:")
        || trimmed.chars().filter(|ch| ch.is_alphabetic()).count() < 20
    {
        return None;
    }

    let without_urls = trimmed
        .split_whitespace()
        .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ");
    (without_urls.chars().filter(|ch| ch.is_alphabetic()).count() >= 20).then_some(without_urls)
}

/// Searches for an ASCII `needle` and returns a byte index valid for
/// `haystack` itself. Lowercasing the haystack first would shift indices,
/// because `str::to_lowercase` is not byte-length preserving.
fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&start| haystack[start..start + needle.len()].eq_ignore_ascii_case(needle))
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len()
        && value.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn parse_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp())
}

fn within_match_window(left: Option<i64>, right: Option<i64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= MATCH_WINDOW_SECONDS as u64,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-08-27T06:00:00Z";

    fn article(id: i64, title: &str, summary: &str) -> IndexedArticle {
        IndexedArticle {
            article_id: id,
            title: title.to_string(),
            summary: Some(summary.to_string()),
            event_at: NOW.to_string(),
        }
    }

    #[test]
    fn action_normalization_clusters_acquisition_reports() {
        let index = ArticleIndex::from_articles(vec![article(
            1,
            "Nvidia closes in on Hugging Face acquisition",
            "Nvidia reportedly agreed to buy Hugging Face for $12.9 billion.",
        )]);

        assert_eq!(
            index.find_duplicate(
                "Nvidia agrees to acquire Hugging Face for $13B",
                Some("The chipmaker has agreed to buy the open-source AI hub."),
                NOW,
            ),
            Some(1)
        );
        assert_eq!(
            index.find_duplicate(
                "Nvidia Buys Hugging Face for $12.9B, Nearly Tripling 2023 Valuation",
                Some("The agreement gives Nvidia control of the model hub."),
                NOW,
            ),
            Some(1)
        );
    }

    #[test]
    fn title_and_summary_cluster_reworded_ai_reports() {
        let index = ArticleIndex::from_articles(vec![
            article(
                1,
                "Anthropic continues compute-gobbling streak in $45B deal with Nscale",
                "The infrastructure agreement gives Anthropic additional AI compute from Nscale.",
            ),
            article(
                2,
                "Google's new AI transcription edits out your ums and ahs",
                "Google updated Gemini Audio. Gemini 3.5 Transcribe automatically removes filler words and supports 85 languages.",
            ),
            article(
                3,
                "The inside story on why OpenAI agents hacked Hugging Face",
                "OpenAI models trained to cheat coordinated an agent hack of Hugging Face.",
            ),
            article(
                4,
                "Nvidia Launches Vera Rubin NVL72 Production With First Operational Racks at Microsoft",
                "Nvidia and Microsoft deployed the first Vera Rubin racks for production.",
            ),
            article(
                5,
                "AI Found Core Lightning Flaw Forces Emergency Node Shutdowns",
                "AI analysis found critical flaws that forced Core Lightning nodes offline.",
            ),
        ]);

        assert_eq!(
            index.find_duplicate(
                "Anthropic Pays Nscale $45B for AI Compute After Microsoft Backs Out",
                Some("Anthropic will rent Nscale data-center capacity for high-end computing."),
                NOW,
            ),
            Some(1)
        );
        assert_eq!(
            index.find_duplicate(
                "Google Ships Gemini 3.5 Transcribe to Cut Transcription Time",
                Some("Google's Gemini 3.5 Transcribe removes filler words and supports 85 languages."),
                NOW,
            ),
            Some(2)
        );
        assert_eq!(
            index.find_duplicate(
                "What We Still Don't Know About OpenAI's Hugging Face Hack",
                Some("OpenAI has not explained why its AI agents went rogue during the Hugging Face breach."),
                NOW,
            ),
            Some(3)
        );
        assert_eq!(
            index.find_duplicate(
                "Nvidia Vera Rubin Racks Hit 30 Times Prior Throughput in First Microsoft Deployment",
                Some("Microsoft deployed Nvidia Vera Rubin racks with far higher throughput."),
                NOW,
            ),
            Some(4)
        );
        assert_eq!(
            index.find_duplicate(
                "AI Finds Critical Core Lightning Flaws, Forcing Emergency Shutdowns",
                Some("The findings forced operators to shut down vulnerable Core Lightning nodes."),
                NOW,
            ),
            Some(5)
        );
    }

    #[test]
    fn same_entity_but_different_events_remain_separate() {
        let index = ArticleIndex::from_articles(vec![
            article(
                1,
                "Nvidia agrees to acquire Hugging Face for $13B",
                "Nvidia will buy the open-source model hub.",
            ),
            article(
                2,
                "Bill Gates proposes robot and token taxes to avert AI job loss",
                "Gates proposed new taxes in response to automation.",
            ),
        ]);

        assert_eq!(
            index.find_duplicate(
                "OpenAI's rogue model incident was worse than we thought",
                Some("OpenAI agents hacked Hugging Face after escaping their test environment."),
                NOW,
            ),
            None
        );
        assert_eq!(
            index.find_duplicate(
                "Bill Gates says we've passed AI's danger thresholds",
                Some("Gates discussed the safety risks posed by artificial intelligence."),
                NOW,
            ),
            None
        );
    }

    #[test]
    fn corpus_false_positives_do_not_cluster() {
        let cases = [
            (
                "NASA's New Space Telescope Is Poised to Discover Hidden Facets of the Universe",
                "The telescope may discover 200,000 planets.",
                "US set for largest mass visa revocation targeting up to 200,000 foreigners",
                "Officials may revoke as many as 200,000 visas.",
            ),
            (
                "Google's Gemini has a branding problem, and so does the rest of AI",
                "Consumer AI apps should stop making users learn product architecture.",
                "Google Ships Gemini 3.5 Transcribe to Cut Transcription Time",
                "Gemini 3.5 Transcribe removes filler words and supports 85 languages.",
            ),
            (
                "Nvidia Launches Vera Rubin Production With First Racks at Microsoft",
                "Nvidia and Microsoft deployed the first production racks.",
                "Xiaomi launches new Xring chip, partners with TSMC for production",
                "Xiaomi will use TSMC to manufacture its newly launched chip.",
            ),
            (
                "EU approves further funding for Ukraine's defence",
                "The European Union approved additional Ukrainian military support.",
                "India approves transfer of missile technology to defence industry",
                "India approved a technology transfer to domestic manufacturers.",
            ),
            (
                "AI Found Core Lightning Flaw Forces Emergency Node Shutdowns",
                "AI-generated findings caused operators to shut down Lightning nodes.",
                "CEO fired developers to make room for AI. Developers create open source AI CEO",
                "Developers responded by releasing an open-source executive agent.",
            ),
            (
                "Motley Fool Promo Code: $200 Off on Stock Advisor September 2026",
                "The latest deal saves subscribers money on Stock Advisor.",
                "Therabody Promo Codes: 15% Off September 2026",
                "Save on massage devices with the latest Therabody deals.",
            ),
            // Overlaps on the affiliate template and the date alone: every word
            // these share is boilerplate, and the brand is what they do not.
            (
                "Hotels.com Coupon Codes for September 2026",
                "Save on bookings with the latest Hotels.com offers.",
                "Hoka Coupon Codes: 30% Off in September 2026",
                "Save on running shoes with the latest Hoka offers.",
            ),
        ];

        for (existing_title, existing_summary, candidate_title, candidate_summary) in cases {
            let index =
                ArticleIndex::from_articles(vec![article(1, existing_title, existing_summary)]);
            assert_eq!(
                index.find_duplicate(candidate_title, Some(candidate_summary), NOW),
                None,
                "unexpected cluster: {existing_title} || {candidate_title}"
            );
        }
    }

    #[test]
    fn boilerplate_summary_falls_back_to_title_and_old_reports_do_not_cluster() {
        let index = ArticleIndex::from_articles(vec![article(
            1,
            "Nvidia closes in on Hugging Face acquisition",
            "https://example.com/story Comments URL: https://example.com/comments Points: 42",
        )]);

        assert_eq!(
            index.find_duplicate(
                "Nvidia agrees to acquire Hugging Face for $13B",
                Some("Article URL: https://example.com/other Comments URL: https://example.com/thread"),
                NOW,
            ),
            Some(1)
        );
        assert_eq!(
            index.find_duplicate(
                "Nvidia agrees to acquire Hugging Face for $13B",
                None,
                "2026-08-20T06:00:00Z",
            ),
            None
        );
    }

    #[test]
    fn summary_boilerplate_is_trimmed_without_byte_index_drift() {
        // "İ" is one char but lowercases to two, so an index taken from a
        // lowercased copy would land mid-character here.
        let summary = "İstanbul transit agency confirmed the rollout schedule today. \
             Comments URL: https://example.com/thread";
        let index = ArticleIndex::from_articles(vec![IndexedArticle {
            article_id: 1,
            title: "Istanbul transit rollout schedule confirmed".to_string(),
            summary: Some(summary.to_string()),
            event_at: NOW.to_string(),
        }]);

        assert_eq!(
            index.find_duplicate(
                "Istanbul transit rollout schedule confirmed by agency",
                Some(summary),
                NOW,
            ),
            Some(1)
        );
    }

    #[test]
    fn pruning_drops_reports_that_can_no_longer_match() {
        let stale = "2026-08-20T06:00:00Z";
        let mut index = ArticleIndex::from_articles(vec![
            IndexedArticle {
                article_id: 1,
                title: "Frontier model safety evaluation published".to_string(),
                summary: None,
                event_at: stale.to_string(),
            },
            IndexedArticle {
                article_id: 2,
                title: "Nvidia agrees to acquire Hugging Face".to_string(),
                summary: None,
                event_at: NOW.to_string(),
            },
        ]);
        assert_eq!(index.len(), 2);

        let cutoff = parse_timestamp(NOW).expect("parse now") - INDEX_RETENTION_SECONDS;
        index.prune_before(cutoff);

        assert_eq!(index.len(), 1);
        assert_eq!(
            index.find_duplicate("Frontier model safety evaluation results", None, NOW),
            None
        );
        assert_eq!(
            index.find_duplicate("Nvidia agrees to acquire Hugging Face for $13B", None, NOW),
            Some(2),
            "surviving entries must stay searchable after pruning"
        );
    }

    #[test]
    fn syndicated_publisher_credits_do_not_affect_matching() {
        let index = ArticleIndex::from_articles(vec![article(
            1,
            "Nvidia agrees to acquire Hugging Face for $13B - The Verge",
            "Nvidia will buy the open-source model hub.",
        )]);

        assert_eq!(
            index.find_duplicate(
                "Nvidia agrees to acquire Hugging Face for $13B | Ars Technica",
                Some("Nvidia will buy the open-source model hub."),
                NOW,
            ),
            Some(1)
        );
        assert_eq!(
            strip_source_attribution("Google ships Gemini 3.5 Transcribe - here's what changed"),
            "Google ships Gemini 3.5 Transcribe - here's what changed",
            "lowercase tails are headline text, not a publisher credit"
        );
        assert_eq!(
            strip_source_attribution("Nvidia - Hugging Face"),
            "Nvidia - Hugging Face",
            "a headline too short to survive the split is left alone"
        );
    }

    #[test]
    fn regular_plurals_share_tokens_with_their_singular() {
        for (plural, singular) in [
            ("racks", "rack"),
            ("flaws", "flaw"),
            ("companies", "company"),
            ("taxes", "tax"),
            ("business", "business"),
            ("analysis", "analysis"),
            ("census", "census"),
        ] {
            assert_eq!(canonical_token(plural), singular, "stemming {plural}");
        }
    }

    #[test]
    fn plural_titles_cluster_with_singular_rewordings() {
        let index = ArticleIndex::from_articles(vec![article(
            1,
            "Regulators approve Broadcom's chip factory expansion",
            "The regulator cleared Broadcom to expand its Arizona chip factory.",
        )]);

        assert_eq!(
            index.find_duplicate(
                "Regulator approves Broadcom chip factories expansion",
                Some("Broadcom won approval to expand its Arizona chip factories."),
                NOW,
            ),
            Some(1)
        );
    }

    #[test]
    fn rolling_back_restores_the_previous_index_state() {
        let mut index = ArticleIndex::from_articles(vec![article(
            1,
            "Nvidia agrees to acquire Hugging Face",
            "Nvidia will buy the open-source model hub.",
        )]);
        let checkpoint = index.checkpoint();
        index.insert(
            2,
            "Anthropic signs $45B compute deal with Nscale",
            Some("Anthropic will rent Nscale data-center capacity."),
            NOW,
        );

        index.rollback_to(checkpoint);

        assert_eq!(index.len(), 1);
        assert_eq!(
            index.find_duplicate(
                "Anthropic pays Nscale $45B for AI compute",
                Some("Anthropic will rent Nscale data-center capacity."),
                NOW,
            ),
            None,
            "rolled-back entries must not match"
        );
        assert_eq!(
            index.find_duplicate(
                "Nvidia agrees to acquire Hugging Face for $13B",
                Some("Nvidia will buy the open-source model hub."),
                NOW,
            ),
            Some(1),
            "entries from before the checkpoint stay searchable"
        );
    }

    #[test]
    fn newly_inserted_entries_are_immediately_searchable() {
        let mut index = ArticleIndex::default();
        index.insert(7, "Frontier model safety evaluation published", None, NOW);

        assert_eq!(
            index.find_duplicate(
                "Frontier model safety evaluation results published",
                None,
                NOW,
            ),
            Some(7)
        );
    }
}
