use std::io::Cursor;
use std::sync::OnceLock;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use feed_rs::model::{Entry, Feed as ParsedFeed};
use feed_rs::parser;

use crate::content::{clean_content, clean_summary};
use crate::db::{FeedCacheValidators, FeedMetadata, NewArticle};

const CONNECT_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const REQUEST_TIMEOUT: StdDuration = StdDuration::from_secs(15);
const MAX_FEED_BYTES: usize = 20 * 1024 * 1024;
const MAX_FETCH_ATTEMPTS: usize = 3;
const RETRY_DELAYS: [StdDuration; MAX_FETCH_ATTEMPTS - 1] =
    [StdDuration::from_millis(250), StdDuration::from_millis(750)];

pub struct FetchedFeed {
    pub metadata: FeedMetadata,
    pub articles: Vec<NewArticle>,
    /// Validators this response served, to be replayed on the next poll.
    pub validators: FeedCacheValidators,
}

pub enum FetchOutcome {
    /// The server confirmed nothing changed since the validators we sent.
    /// There is no body to parse and nothing to insert.
    NotModified,
    Fetched(FetchedFeed),
}

pub async fn fetch_feed(url: &str) -> Result<FetchedFeed, String> {
    match fetch_feed_conditional(url, &FeedCacheValidators::default()).await? {
        FetchOutcome::Fetched(fetched) => Ok(fetched),
        // Unreachable: an unconditional request cannot be answered with 304.
        FetchOutcome::NotModified => Err("Feed returned no content".to_string()),
    }
}

pub async fn fetch_feed_conditional(
    url: &str,
    validators: &FeedCacheValidators,
) -> Result<FetchOutcome, String> {
    for attempt in 0..MAX_FETCH_ATTEMPTS {
        let mut request = http_client().get(url).header("User-Agent", "Xone/0.1");
        if let Some(etag) = &validators.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &validators.last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) if has_retry(attempt) && is_retryable_request_error(&err) => {
                wait_before_retry(attempt).await;
                continue;
            }
            Err(err) => return Err(format!("Could not fetch feed: {}", err.without_url())),
        };

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified);
        }
        if response.status().is_server_error() && has_retry(attempt) {
            wait_before_retry(attempt).await;
            continue;
        }

        let response = response
            .error_for_status()
            .map_err(|err| format!("Feed returned an error: {}", err.without_url()))?;
        let served_validators = response_validators(response.headers());

        let bytes = match read_feed_bytes(response, MAX_FEED_BYTES).await {
            Ok(bytes) => bytes,
            Err(FeedBodyError::Request(err)) if has_retry(attempt) && (err.is_timeout() || err.is_body()) => {
                wait_before_retry(attempt).await;
                continue;
            }
            Err(FeedBodyError::Request(err)) => return Err(format!("Could not read feed body: {}", err.without_url())),
            Err(FeedBodyError::TooLarge) => return Err("Feed exceeds the 20 MiB download limit".to_string()),
        };
        let parsed =
            parser::parse(Cursor::new(bytes)).map_err(|err| format!("Invalid feed: {err}"))?;

        let metadata = feed_metadata(url, &parsed);
        let articles = feed_articles(&parsed);

        return Ok(FetchOutcome::Fetched(FetchedFeed {
            metadata,
            articles,
            validators: served_validators,
        }));
    }

    unreachable!("feed attempts always return on their final iteration")
}

#[derive(Debug)]
enum FeedBodyError {
    Request(reqwest::Error),
    TooLarge,
}

async fn read_feed_bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, FeedBodyError> {
    if response.content_length().is_some_and(|size| size > limit as u64) {
        return Err(FeedBodyError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(FeedBodyError::Request)? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(FeedBodyError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn has_retry(attempt: usize) -> bool {
    attempt + 1 < MAX_FETCH_ATTEMPTS
}

fn is_retryable_request_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || (error.is_request() && !error.is_builder() && !error.is_redirect())
}

async fn wait_before_retry(attempt: usize) {
    tokio::time::sleep(RETRY_DELAYS[attempt]).await;
}

fn response_validators(headers: &reqwest::header::HeaderMap) -> FeedCacheValidators {
    let header = |name: reqwest::header::HeaderName| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    FeedCacheValidators {
        etag: header(reqwest::header::ETAG),
        last_modified: header(reqwest::header::LAST_MODIFIED),
    }
}

fn feed_articles(parsed: &ParsedFeed) -> Vec<NewArticle> {
    // A feed that dates its other entries but not this one is malformed, and
    // an undated entry would otherwise bypass the freshness rule and be filed
    // under the current time. Feeds that never publish dates keep working.
    let feed_dates_entries = parsed
        .entries
        .iter()
        .any(|entry| entry.published.or(entry.updated).is_some());
    parsed
        .entries
        .iter()
        .filter_map(|entry| entry_article(entry, feed_dates_entries))
        .collect()
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("valid RSS HTTP client")
    })
}

fn feed_metadata(url: &str, feed: &ParsedFeed) -> FeedMetadata {
    FeedMetadata {
        url: url.to_string(),
        title: feed
            .title
            .as_ref()
            .map(|title| title.content.trim().to_string())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| url.to_string()),
        description: feed
            .description
            .as_ref()
            .map(|description| clean_summary(&description.content))
            .filter(|description| !description.is_empty()),
        site_url: feed.links.first().map(|link| link.href.clone()),
    }
}

fn entry_article(entry: &Entry, require_published_date: bool) -> Option<NewArticle> {
    let title = entry
        .title
        .as_ref()
        .map(|title| title.content.trim().to_string())
        .filter(|title| !title.is_empty())?;
    let url = entry.links.first()?.href.clone();

    let published = entry.published.or(entry.updated);
    match published {
        Some(published_at) => {
            let now = Utc::now();
            if published_at > now || published_at < now - Duration::hours(96) {
                return None;
            }
        }
        None if require_published_date => return None,
        None => {}
    }

    let raw_summary = entry
        .summary
        .as_ref()
        .map(|summary| summary.content.as_str())
        .unwrap_or("");
    let raw_content = entry
        .content
        .as_ref()
        .and_then(|content| content.body.as_deref())
        .unwrap_or(raw_summary);

    let summary = clean_summary(raw_summary);
    let content = clean_content(raw_content);

    Some(NewArticle {
        title,
        content: if content.is_empty() {
            summary.clone()
        } else {
            content
        },
        summary: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        url,
        author: entry.authors.first().map(|author| author.name.clone()),
        published_at: published.map(|date| date.to_rfc3339()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration as TestDuration, Instant};

    fn rss(items: &str) -> ParsedFeed {
        let xml =
            format!("<rss version=\"2.0\"><channel><title>Test</title>{items}</channel></rss>");
        parser::parse(Cursor::new(xml.into_bytes())).expect("parse test feed")
    }

    fn item(title: &str, published: Option<&str>) -> String {
        let date = published
            .map(|date| format!("<pubDate>{date}</pubDate>"))
            .unwrap_or_default();
        format!("<item><title>{title}</title><link>https://example.com/{title}</link>{date}</item>")
    }

    fn test_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn serve_responses(
        responses: Vec<String>,
        linger: TestDuration,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test feed server");
        listener
            .set_nonblocking(true)
            .expect("set test server nonblocking");
        let address = listener.local_addr().expect("read test server address");
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + linger;
            let mut requests = Vec::new();

            while requests.len() < responses.len() && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(TestDuration::from_secs(1)))
                            .expect("set request read timeout");
                        let mut request = Vec::new();
                        let mut buffer = [0_u8; 1024];
                        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                            let read = stream.read(&mut buffer).expect("read test request");
                            if read == 0 {
                                break;
                            }
                            request.extend_from_slice(&buffer[..read]);
                        }
                        let response = &responses[requests.len()];
                        requests.push(String::from_utf8_lossy(&request).into_owned());
                        stream
                            .write_all(response.as_bytes())
                            .expect("write test response");
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(TestDuration::from_millis(10));
                    }
                    Err(err) => panic!("accept test request: {err}"),
                }
            }

            requests
        });

        (format!("http://{address}/feed.xml"), handle)
    }

    #[tokio::test]
    async fn limits_feed_bodies_with_and_without_content_length() {
        for response in [
            test_response("200 OK", "0123456789"),
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nA\r\n0123456789\r\n0\r\n\r\n".to_string(),
        ] {
            let (url, server) = serve_responses(vec![response], TestDuration::from_secs(2));
            let response = http_client().get(url).send().await.unwrap();
            assert!(matches!(read_feed_bytes(response, 8).await, Err(FeedBodyError::TooLarge)));
            server.join().unwrap();
        }
        let (url, server) = serve_responses(vec![test_response("200 OK", "small")], TestDuration::from_secs(2));
        let response = http_client().get(url).send().await.unwrap();
        assert_eq!(read_feed_bytes(response, 8).await.unwrap(), b"small");
        server.join().unwrap();
    }

    #[test]
    fn rejects_excessive_xml_namespaces() {
        let namespaces = (0..300).map(|n| format!(" xmlns:n{n}=\"urn:{n}\"")).collect::<String>();
        let xml = format!("<rss version=\"2.0\"{namespaces}><channel><title>Test</title></channel></rss>");
        assert!(parser::parse(Cursor::new(xml)).is_err());
    }

    #[tokio::test]
    async fn transient_server_errors_are_retried_with_conditional_headers() {
        let feed = "<rss version=\"2.0\"><channel><title>Recovered</title><item><title>Story</title><link>https://example.com/story</link></item></channel></rss>";
        let (url, server) = serve_responses(
            vec![
                test_response("503 Service Unavailable", "temporary"),
                test_response("502 Bad Gateway", "temporary"),
                test_response("200 OK", feed),
            ],
            TestDuration::from_secs(4),
        );
        let validators = FeedCacheValidators {
            etag: Some("\"test-etag\"".to_string()),
            last_modified: Some("Fri, 05 Sep 2026 14:22:00 GMT".to_string()),
        };

        let result = fetch_feed_conditional(&url, &validators).await;
        let requests = server.join().expect("join test feed server");

        assert!(result.is_ok(), "third response should recover");
        assert_eq!(requests.len(), 3);
        for request in requests {
            let request = request.to_ascii_lowercase();
            assert!(request.contains("user-agent: xone/0.1"));
            assert!(request.contains("if-none-match: \"test-etag\""));
            assert!(request.contains("if-modified-since: fri, 05 sep 2026 14:22:00 gmt"));
        }
    }

    #[tokio::test]
    async fn transient_server_errors_stop_after_two_retries() {
        let (url, server) = serve_responses(
            vec![
                test_response("503 Service Unavailable", "temporary"),
                test_response("503 Service Unavailable", "temporary"),
                test_response("503 Service Unavailable", "temporary"),
            ],
            TestDuration::from_secs(4),
        );

        let result = fetch_feed(&url).await;
        let requests = server.join().expect("join test feed server");

        assert_eq!(requests.len(), 3);
        let error = match result {
            Ok(_) => panic!("third server error must be returned"),
            Err(error) => error,
        };
        assert!(error.contains("503 Service Unavailable"));
    }

    #[tokio::test]
    async fn interrupted_request_is_retried() {
        let feed = "<rss version=\"2.0\"><channel><title>Recovered</title></channel></rss>";
        let (url, server) = serve_responses(
            vec![String::new(), test_response("200 OK", feed)],
            TestDuration::from_secs(4),
        );

        let result = fetch_feed(&url).await;
        let requests = server.join().expect("join test feed server");

        assert_eq!(requests.len(), 2);
        assert!(result.is_ok(), "second request should recover");
    }

    #[tokio::test]
    async fn permanent_client_errors_are_not_retried() {
        let feed = "<rss version=\"2.0\"><channel><title>Must not reach</title></channel></rss>";
        let (url, server) = serve_responses(
            vec![
                test_response("403 Forbidden", "blocked"),
                test_response("200 OK", feed),
            ],
            TestDuration::from_millis(800),
        );

        let result = fetch_feed(&url).await;
        let requests = server.join().expect("join test feed server");

        assert_eq!(requests.len(), 1);
        let error = match result {
            Ok(_) => panic!("client error must be returned"),
            Err(error) => error,
        };
        assert!(error.contains("403 Forbidden"));
    }

    #[test]
    fn validators_are_read_from_the_response_and_blank_values_ignored() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ETAG, "\"a3f5c9\"".parse().unwrap());
        headers.insert(
            reqwest::header::LAST_MODIFIED,
            "Fri, 05 Sep 2026 14:22:00 GMT".parse().unwrap(),
        );

        let validators = response_validators(&headers);
        assert_eq!(validators.etag.as_deref(), Some("\"a3f5c9\""));
        assert_eq!(
            validators.last_modified.as_deref(),
            Some("Fri, 05 Sep 2026 14:22:00 GMT")
        );
        assert!(!validators.is_empty());

        // A publisher that sends the headers but leaves them blank must not
        // produce a validator we would then replay as a match.
        let mut blank = reqwest::header::HeaderMap::new();
        blank.insert(reqwest::header::ETAG, "   ".parse().unwrap());
        assert!(response_validators(&blank).is_empty());
        assert!(response_validators(&reqwest::header::HeaderMap::new()).is_empty());
    }

    #[test]
    fn stale_and_future_entries_are_dropped() {
        let fresh = (Utc::now() - Duration::hours(2)).to_rfc2822();
        let stale = (Utc::now() - Duration::hours(120)).to_rfc2822();
        let future = (Utc::now() + Duration::hours(5)).to_rfc2822();
        let parsed = rss(&format!(
            "{}{}{}",
            item("fresh", Some(&fresh)),
            item("stale", Some(&stale)),
            item("future", Some(&future))
        ));

        let titles = feed_articles(&parsed)
            .into_iter()
            .map(|article| article.title)
            .collect::<Vec<_>>();

        assert_eq!(titles, vec!["fresh".to_string()]);
    }

    #[test]
    fn undated_entries_are_dropped_only_when_the_feed_dates_others() {
        let fresh = (Utc::now() - Duration::hours(2)).to_rfc2822();
        let mixed = rss(&format!(
            "{}{}",
            item("dated", Some(&fresh)),
            item("undated", None)
        ));

        let titles = feed_articles(&mixed)
            .into_iter()
            .map(|article| article.title)
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["dated".to_string()]);

        let dateless = rss(&format!("{}{}", item("first", None), item("second", None)));
        let articles = feed_articles(&dateless);

        assert_eq!(articles.len(), 2, "date-less feeds must keep working");
        assert!(articles
            .iter()
            .all(|article| article.published_at.is_none()));
    }
}
