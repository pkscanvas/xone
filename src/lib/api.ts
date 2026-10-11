import { invoke } from "@tauri-apps/api/core";
import type {
  AiSettings,
  AiModel,
  AiStatus,
  AppSettings,
  ArticleRetentionDays,
  Article,
  ArticleOpenResult,
  ClassifyZoneResult,
  Feed,
  FilterBriefAnswer,
  FilterBriefAssistResult,
  FocusLevel,
  ReadContext,
  SmartFeedCounts,
  TldrEntry,
  Zone
} from "./types";

export function listFeeds(): Promise<Feed[]> {
  return invoke<Feed[]>("list_feeds");
}

export function addFeed(
  url: string,
  displayName: string | null,
  category: string,
  region: string | null
): Promise<Feed> {
  return invoke<Feed>("add_feed", { url, displayName, category, region });
}

export function refreshFeed(id: number): Promise<number> {
  return invoke<number>("refresh_feed", { id });
}

export function removeFeed(id: number): Promise<void> {
  return invoke<void>("remove_feed", { id });
}

export function getAllArticles(feedId: number): Promise<Article[]> {
  return invoke<Article[]>("get_all_articles", { feedId });
}

export function getSmartFeedCounts(): Promise<SmartFeedCounts> {
  return invoke<SmartFeedCounts>("get_smart_feed_counts");
}

export function getTodayArticles(): Promise<Article[]> {
  return invoke<Article[]>("get_today_articles");
}

export function getUnreadArticles(): Promise<Article[]> {
  return invoke<Article[]>("get_unread_articles");
}

export function listZones(): Promise<Zone[]> {
  return invoke<Zone[]>("list_zones");
}

export function createZone(
  name: string,
  filterDescription: string,
  focusLevel: FocusLevel
): Promise<Zone> {
  return invoke<Zone>("create_zone", { name, filterDescription, focusLevel });
}

export function updateZone(
  id: number,
  name: string,
  filterDescription: string,
  focusLevel: FocusLevel
): Promise<Zone> {
  return invoke<Zone>("update_zone", { id, name, filterDescription, focusLevel });
}

export function deleteZone(id: number): Promise<void> {
  return invoke<void>("delete_zone", { id });
}

export function reorderZones(orderedIds: number[]): Promise<void> {
  return invoke<void>("reorder_zones", { orderedIds });
}

export function assignFeed(zoneId: number, feedId: number): Promise<void> {
  return invoke<void>("assign_feed", { zoneId, feedId });
}

export function unassignFeed(zoneId: number, feedId: number): Promise<void> {
  return invoke<void>("unassign_feed", { zoneId, feedId });
}

export function getZoneFeeds(zoneId: number): Promise<Feed[]> {
  return invoke<Feed[]>("get_zone_feeds", { zoneId });
}

export function getZoneArticles(zoneId: number): Promise<Article[]> {
  return invoke<Article[]>("get_zone_articles", { zoneId });
}

export function markRead(id: number, context: ReadContext): Promise<ArticleOpenResult> {
  return invoke<ArticleOpenResult>("mark_read", { id, context });
}

export function retryZoneBehavior(zoneId: number): Promise<void> {
  return invoke<void>("retry_zone_behavior", { zoneId });
}

export function resetZoneBehavior(zoneId: number): Promise<Zone> {
  return invoke<Zone>("reset_zone_behavior", { zoneId });
}

export function getArticleCleanupCount(retentionDays: ArticleRetentionDays): Promise<number> {
  return invoke<number>("get_article_cleanup_count", { retentionDays });
}

export function cleanupOldArticles(retentionDays: ArticleRetentionDays): Promise<number> {
  return invoke<number>("cleanup_old_articles", { retentionDays });
}

export function getAppSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_app_settings");
}

export function updateAppSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("update_app_settings", { settings });
}

export function getAiSettings(): Promise<AiSettings> {
  return invoke<AiSettings>("get_ai_settings");
}

export function updateAiSettings(settings: AiSettings): Promise<AiSettings> {
  return invoke<AiSettings>("update_ai_settings", { settings });
}

export function getAiStatus(): Promise<AiStatus> {
  return invoke<AiStatus>("get_ai_status");
}

export function checkAiStatus(settings: AiSettings): Promise<AiStatus> {
  return invoke<AiStatus>("check_ai_status", { settings });
}

export function fetchAiModels(settings: AiSettings): Promise<AiModel[]> {
  return invoke<AiModel[]>("fetch_ai_models", { settings });
}

export function recommendSources(
  filterDescription: string,
  excludedFeedIds: number[],
  limit: number
): Promise<number[]> {
  return invoke<number[]>("recommend_sources", { filterDescription, excludedFeedIds, limit });
}

export function assistFilterBrief(
  filterDescription: string,
  clarifications: FilterBriefAnswer[]
): Promise<FilterBriefAssistResult> {
  return invoke<FilterBriefAssistResult>("assist_filter_brief", {
    filterDescription,
    clarifications
  });
}

export function classifyZone(
  zoneId: number,
  reclassify: boolean,
  processingLimit: number
): Promise<ClassifyZoneResult> {
  return invoke<ClassifyZoneResult>("classify_zone", { zoneId, reclassify, processingLimit });
}

export function openInBrowser(url: string): Promise<void> {
  return invoke<void>("open_in_browser", { url });
}

export function getTldr(): Promise<TldrEntry[]> {
  return invoke<TldrEntry[]>("get_tldr");
}

export function regenerateTldr(zoneId: number | null = null): Promise<void> {
  return invoke<void>("regenerate_tldr", { zoneId });
}
