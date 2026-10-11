export interface Feed {
  id: number;
  url: string;
  title: string;
  description: string | null;
  site_url: string | null;
  category: string | null;
  region: string | null;
  last_fetched_at: string | null;
  created_at: string;
  article_count: number;
}

export interface Article {
  id: number;
  feed_id: number;
  title: string;
  content: string;
  summary: string | null;
  url: string;
  author: string | null;
  published_at: string | null;
  is_read: boolean;
  fetched_at: string;
  ai_match_level: MatchLevel | null;
  related_articles: RelatedArticle[];
}

export interface RelatedArticle {
  feed_id: number;
  title: string;
  summary: string | null;
  url: string;
  published_at: string | null;
  fetched_at: string;
}

export type MatchLevel = "no" | "low" | "medium" | "high";
export type FocusLevel = "broad" | "focused" | "laser";

export type SmartFeedKind = "today" | "unread" | "tldr";

export interface TldrSentence {
  text: string;
  article_ids: number[];
}

export interface TldrEntry {
  zone_id: number;
  sentences: TldrSentence[];
  citations: Article[];
  generated_at: string | null;
  window_start: string | null;
  window_end: string | null;
  article_count: number;
  status: "ready" | "updating" | "empty" | "error" | "unconfigured";
  stale: boolean;
  error: string | null;
}

export interface SmartFeedCounts {
  today: number;
  all_unread: number;
}

export interface Zone {
  id: number;
  name: string;
  filter_description: string;
  focus_level: FocusLevel;
  created_at: string;
  unread_count: number;
  last_updated_at: string | null;
  behavior_profile: string;
  behavior_reads_processed: number;
  behavior_reads_pending: number;
  behavior_updated_at: string | null;
  behavior_last_error: string | null;
}

export type ReadContext =
  | { kind: "zone"; zone_id: number }
  | { kind: "smart_feed" }
  | { kind: "other" };

export interface ArticleOpenResult {
  newly_read: boolean;
  unread_zone_ids: number[];
  behavior_progress: Array<{
    zone_id: number;
    pending_reads: number;
  }>;
}

export interface BehaviorUpdate {
  zone_id: number;
  phase: "completed" | "failed" | "reset";
  zone: Zone | null;
}

export interface AppSettings {
  refresh_interval_minutes: number;
  classification_processing_mode: ClassificationProcessingMode;
  classification_processing_limit: number;
  article_retention_days: ArticleRetentionDays;
}

export interface AppearanceSettings {
  reading_font: ReadingFont;
  text_size: TextSize;
}

export type ReadingFont = "editorial" | "mono" | "digital";
export type TextSize = "small" | "standard" | "large";

export type ClassificationProcessingMode = "all_unclassified" | "limited";
export type ArticleRetentionDays = 0 | 7 | 14 | 30 | 90;

export type AiProvider =
  | "ollama"
  | "lmstudio"
  | "openai"
  | "anthropic"
  | "groq"
  | "openrouter"
  | "custom";

export interface AiProviderConfig {
  api_key: string;
  model: string;
  base_url: string;
  max_tokens: number | null;
  temperature: number | null;
  top_p: number | null;
}

export interface AiSettings {
  provider: AiProvider;
  providers: Record<AiProvider, AiProviderConfig>;
}

export interface AiModel {
  id: string;
  label: string;
  details: string | null;
}

export interface AiStatus {
  connected: boolean;
  provider: string;
  message: string;
}

export interface FilterBriefQuestion {
  question: string;
  options: string[];
}

export interface FilterBriefAnswer {
  question: string;
  selected_options: string[];
}

export interface FilterBriefAssistResult {
  brief: string;
  questions: FilterBriefQuestion[];
}

export interface UserError {
  message: string;
  detail?: string;
}

export interface ClassifyZoneResult {
  classified: number;
  relevant: number;
  sources_refreshed: number;
  articles_fetched: number;
  articles_classified: number;
  articles_relevant: number;
  articles_skipped_by_limit: number;
  /** Left for the background drain because this pass hit its burst cap. */
  articles_pending: number;
  processing_limit: number;
  failures: ClassifyZoneFailure[];
}

export interface ClassifyZoneFailure {
  source: string;
  message: string;
}

export interface AutoRefreshStatus {
  phase: "processing_zones" | "completed" | "failed";
  started_at: string;
  completed_at: string | null;
  sources_total: number;
  sources_completed: number;
  sources_refreshed: number;
  articles_fetched: number;
  articles_inserted: number;
  zones_total: number;
  zones_completed: number;
  articles_classified: number;
  articles_relevant: number;
  articles_queued: number;
  processing_limit: number;
  /** True when this status describes the paced backlog drain, not a cycle. */
  is_drain: boolean;
  failures: ClassifyZoneFailure[];
}
