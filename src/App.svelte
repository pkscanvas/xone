<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import AddFeed from "./components/AddFeed.svelte";
  import TldrPage from "./components/TldrPage.svelte";
  import ArticleList from "./components/ArticleList.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";
  import Settings from "./components/Settings.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import SourcesPage from "./components/SourcesPage.svelte";
  import ZoneEditor from "./components/ZoneEditor.svelte";
  import {
    addFeed,
    assistFilterBrief,
    assignFeed,
    classifyZone,
    cleanupOldArticles,
    createZone,
    deleteZone,
    fetchAiModels,
    getAppSettings,
    getAiSettings,
    getAiStatus,
    getAllArticles,
    getArticleCleanupCount,
    getSmartFeedCounts,
    getTldr,
    regenerateTldr,
    getTodayArticles,
    getUnreadArticles,
    getZoneArticles,
    getZoneFeeds,
    listFeeds,
    listZones,
    markRead,
    openInBrowser,
    recommendSources,
    refreshFeed,
    removeFeed,
    reorderZones,
    resetZoneBehavior,
    retryZoneBehavior,
    unassignFeed,
    updateAppSettings,
    updateAiSettings,
    updateZone
  } from "./lib/api";
  import type {
    AiSettings,
    AiModel,
    AiStatus,
    AppSettings,
    AppearanceSettings,
    Article,
    ArticleRetentionDays,
    AutoRefreshStatus,
    BehaviorUpdate,
    ClassifyZoneResult,
    Feed,
    FocusLevel,
    ReadContext,
    SmartFeedCounts,
    SmartFeedKind,
    TldrEntry,
    UserError,
    Zone
  } from "./lib/types";

  type ViewKind = "none" | "feed" | "zone" | "smart" | "sources" | "tldr";
  type ThemeMode = "light" | "dark";
  type DeleteRequest =
    | { kind: "source"; item: Feed }
    | { kind: "zone"; item: Zone }
    | { kind: "behavior"; item: Zone };
  const SOURCE_LIMIT = 10;
  const ALL_UNCLASSIFIED = 0;
  const REFRESH_ALL_CONCURRENCY = 4;
  const THEME_STORAGE_KEY = "zonereader-theme";
  const APPEARANCE_STORAGE_KEY = "zonereader-appearance";
  const DEFAULT_APPEARANCE: AppearanceSettings = {
    reading_font: "editorial",
    text_size: "standard"
  };
  const DEFAULT_SIDEBAR_WIDTH = 272;
  const MIN_SIDEBAR_WIDTH = 220;
  const MAX_SIDEBAR_WIDTH = 480;
  const SIDEBAR_KEYBOARD_STEP = 16;

  // Collections are always replaced wholesale, never mutated in place, so
  // `$state.raw` keeps large article/source lists free of proxy overhead.
  let feeds = $state.raw<Feed[]>([]);
  let zones = $state.raw<Zone[]>([]);
  let smartFeedCounts = $state.raw<SmartFeedCounts>({ today: 0, all_unread: 0 });
  let articles = $state.raw<Article[]>([]);
  let tldrEntries = $state.raw<TldrEntry[]>([]);
  let tldrError = $state.raw<UserError | null>(null);
  let tldrLoading = $state(false);
  let tldrRequest = 0;
  let selectedFeed = $state.raw<Feed | null>(null);
  let selectedZone = $state.raw<Zone | null>(null);
  let selectedSmartFeed = $state<SmartFeedKind | null>(null);
  let selectedArticle = $state.raw<Article | null>(null);
  let viewKind = $state<ViewKind>("none");
  let feedsLoading = $state(true);
  let articlesLoading = $state(false);
  let addFeedOpen = $state(false);
  let addFeedLoading = $state(false);
  let addFeedError = $state.raw<UserError | null>(null);
  let zoneEditorOpen = $state(false);
  let zoneEditorLoading = $state(false);
  let zoneEditorError = $state.raw<UserError | null>(null);
  let zoneEditorZone = $state.raw<Zone | null>(null);
  let zoneEditorFeedIds = $state.raw<number[]>([]);
  let settingsOpen = $state(false);
  let settingsError = $state.raw<UserError | null>(null);
  let aiSettings = $state.raw<AiSettings | null>(null);
  let appSettings = $state.raw<AppSettings | null>(null);
  let aiStatus = $state.raw<AiStatus | null>(null);
  let classifyingZone = $state(false);
  let focusSaving = $state(false);
  let busyFeedId = $state<number | null>(null);
  let refreshingAll = $state(false);
  let selectedZoneFeedCount = $state<number | null>(null);
  let sourceNames = $derived(new Map(feeds.map((feed) => [feed.id, feed.title])));
  let reclassifyResult = $state.raw<ClassifyZoneResult | null>(null);
  let deleteRequest = $state.raw<DeleteRequest | null>(null);
  let deleteLoading = $state(false);
  let error = $state.raw<UserError | null>(null);
  let theme = $state<ThemeMode>("light");
  let appearance = $state.raw<AppearanceSettings>({ ...DEFAULT_APPEARANCE });
  let autoRefreshStatus = $state.raw<AutoRefreshStatus | null>(null);
  let sidebarWidth = $state(DEFAULT_SIDEBAR_WIDTH);
  // Pointer bookkeeping is only ever read inside the resize handlers, so it
  // stays out of the reactive graph.
  let sidebarResizePointerId: number | null = null;
  let sidebarResizeStartX = 0;
  let sidebarResizeStartWidth = DEFAULT_SIDEBAR_WIDTH;

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let unlistenBehavior: (() => void) | undefined;
    let unlistenTldr: (() => void) | undefined;

    void listen<AutoRefreshStatus>("auto-refresh-status", (event) => {
      void handleAutoRefreshStatus(event.payload);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });

    void listen<BehaviorUpdate>("zone-behavior-updated", (event) => {
      void handleBehaviorUpdate(event.payload);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlistenBehavior = cleanup;
    });

    void listen<number>("tldr-updated", () => {
      if (viewKind === "tldr") void loadTldr();
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlistenTldr = cleanup;
    });

    void initializeApp();

    return () => {
      disposed = true;
      unlisten?.();
      unlistenBehavior?.();
      unlistenTldr?.();
    };
  });

  async function initializeApp() {
    applyTheme(initialTheme(), false);
    applyAppearance(initialAppearance(), false);
    await Promise.all([loadFeedsAndZones(), loadAppSettings(), loadAiSettings()]);
    void refreshAiStatus(false);
    if (zones.length > 0) {
      await selectZone(zones[0]);
    }
    if (zones.length === 0) openSources();
  }

  function initialTheme(): ThemeMode {
    const savedTheme = localStorage.getItem(THEME_STORAGE_KEY);
    if (savedTheme === "light" || savedTheme === "dark") return savedTheme;
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  function applyTheme(nextTheme: ThemeMode, persist = true) {
    theme = nextTheme;
    document.documentElement.dataset.theme = nextTheme;
    if (persist) localStorage.setItem(THEME_STORAGE_KEY, nextTheme);
  }

  function toggleTheme() {
    applyTheme(theme === "dark" ? "light" : "dark");
  }

  function initialAppearance(): AppearanceSettings {
    try {
      const saved = JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY) ?? "null");
      return {
        reading_font: ["editorial", "mono", "digital"].includes(saved?.reading_font)
          ? saved.reading_font
          : DEFAULT_APPEARANCE.reading_font,
        text_size: ["small", "standard", "large"].includes(saved?.text_size)
          ? saved.text_size
          : DEFAULT_APPEARANCE.text_size
      };
    } catch {
      return { ...DEFAULT_APPEARANCE };
    }
  }

  function renderAppearance(nextAppearance: AppearanceSettings) {
    document.documentElement.dataset.readingFont = nextAppearance.reading_font;
    document.documentElement.dataset.textSize = nextAppearance.text_size;
  }

  function previewAppearance(nextAppearance: AppearanceSettings) {
    renderAppearance(nextAppearance);
  }

  function applyAppearance(nextAppearance: AppearanceSettings, persist = true) {
    appearance = { ...nextAppearance };
    renderAppearance(nextAppearance);
    if (persist) localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(nextAppearance));
  }

  function clampSidebarWidth(width: number): number {
    return Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, width));
  }

  function startSidebarResize(event: PointerEvent) {
    if (event.button !== 0) return;
    const divider = event.currentTarget as HTMLElement;
    sidebarResizePointerId = event.pointerId;
    sidebarResizeStartX = event.clientX;
    sidebarResizeStartWidth = sidebarWidth;
    divider.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function resizeSidebar(event: PointerEvent) {
    if (event.pointerId !== sidebarResizePointerId) return;
    sidebarWidth = clampSidebarWidth(
      sidebarResizeStartWidth + event.clientX - sidebarResizeStartX
    );
  }

  function stopSidebarResize(event: PointerEvent) {
    if (event.pointerId !== sidebarResizePointerId) return;
    const divider = event.currentTarget as HTMLElement;
    if (divider.hasPointerCapture(event.pointerId)) {
      divider.releasePointerCapture(event.pointerId);
    }
    sidebarResizePointerId = null;
  }

  function resizeSidebarWithKeyboard(event: KeyboardEvent) {
    let nextWidth = sidebarWidth;
    if (event.key === "ArrowLeft") nextWidth -= SIDEBAR_KEYBOARD_STEP;
    else if (event.key === "ArrowRight") nextWidth += SIDEBAR_KEYBOARD_STEP;
    else if (event.key === "Home") nextWidth = MIN_SIDEBAR_WIDTH;
    else if (event.key === "End") nextWidth = MAX_SIDEBAR_WIDTH;
    else return;

    event.preventDefault();
    sidebarWidth = clampSidebarWidth(nextWidth);
  }

  async function loadFeedsAndZones(showLoading = true) {
    if (showLoading) feedsLoading = true;
    error = null;
    try {
      [feeds, zones, smartFeedCounts] = await Promise.all([
        listFeeds(),
        listZones(),
        getSmartFeedCounts()
      ]);
    } catch (err) {
      error = userError("Could not load sources and Xones.", err);
    } finally {
      if (showLoading) feedsLoading = false;
    }
  }

  async function loadAppSettings() {
    try {
      appSettings = await getAppSettings();
    } catch (err) {
      error = userError("Could not load app settings.", err);
    }
  }

  async function saveZoneOrder(orderedZoneIds: number[]) {
    const previousZones = zones;
    const zonesById = new Map(zones.map((zone) => [zone.id, zone]));
    const reorderedZones = orderedZoneIds
      .map((zoneId) => zonesById.get(zoneId))
      .filter((zone): zone is Zone => zone !== undefined);
    if (reorderedZones.length !== zones.length) return;

    zones = reorderedZones;
    try {
      await reorderZones(orderedZoneIds);
    } catch (err) {
      zones = previousZones;
      error = userError("Could not save the Xone order.", err);
    }
  }

  async function loadAiSettings() {
    try {
      aiSettings = await getAiSettings();
    } catch (err) {
      error = userError("Could not load AI provider settings.", err);
    }
  }

  async function selectFeed(feed: Feed) {
    viewKind = "feed";
    selectedFeed = feed;
    selectedZone = null;
    selectedSmartFeed = null;
    selectedZoneFeedCount = null;
    reclassifyResult = null;
    selectedArticle = null;
    await loadFeedArticles(feed.id);

    if (articles.length === 0 && !feed.last_fetched_at) {
      await refreshSelectedFeed();
    }
  }

  function openSources() {
    viewKind = "sources";
    selectedFeed = null;
    selectedZone = null;
    selectedSmartFeed = null;
    selectedZoneFeedCount = null;
    reclassifyResult = null;
    selectedArticle = null;
    articles = [];
  }

  async function selectZone(zone: Zone) {
    viewKind = "zone";
    selectedZone = zone;
    selectedFeed = null;
    selectedSmartFeed = null;
    selectedZoneFeedCount = null;
    reclassifyResult = null;
    selectedArticle = null;
    await loadZoneArticles(zone.id);
  }

  async function selectSmartFeed(smartFeed: SmartFeedKind) {
    viewKind = smartFeed === "tldr" ? "tldr" : "smart";
    selectedSmartFeed = smartFeed;
    selectedFeed = null;
    selectedZone = null;
    selectedZoneFeedCount = null;
    reclassifyResult = null;
    selectedArticle = null;
    await loadSmartFeedArticles(smartFeed);
  }

  async function loadFeedArticles(feedId: number) {
    articlesLoading = true;
    error = null;
    try {
      articles = await getAllArticles(feedId);
      selectedArticle = null;
    } catch (err) {
      error = userError("Could not load articles.", err);
      articles = [];
    } finally {
      articlesLoading = false;
    }
  }

  async function loadZoneArticles(zoneId: number) {
    articlesLoading = true;
    error = null;
    try {
      const [zoneArticles, assignedFeeds] = await Promise.all([
        getZoneArticles(zoneId),
        getZoneFeeds(zoneId)
      ]);
      articles = zoneArticles;
      selectedZoneFeedCount = assignedFeeds.length;
      selectedArticle = null;
    } catch (err) {
      error = userError("Could not load Xone articles.", err);
      articles = [];
      selectedZoneFeedCount = null;
    } finally {
      articlesLoading = false;
    }
  }

  async function loadTldr() {
    const request = ++tldrRequest;
    tldrLoading = true;
    try {
      const entries = await getTldr();
      if (request === tldrRequest) {
        tldrEntries = entries;
        tldrError = null;
      }
    } catch (err) {
      if (request === tldrRequest) tldrError = userError("Could not load briefings.", err);
    } finally {
      if (request === tldrRequest) tldrLoading = false;
    }
  }

  async function rebuildTldr(zoneId: number | null = null) {
    try {
      await regenerateTldr(zoneId);
      await loadTldr();
    } catch (err) {
      tldrError = userError("Could not update briefings.", err);
    }
  }

  async function loadSmartFeedArticles(smartFeed: SmartFeedKind) {
    if (smartFeed === "tldr") {
      articles = [];
      await loadTldr();
      return;
    }
    articlesLoading = true;
    error = null;
    try {
      const [smartArticles, latestCounts] = await Promise.all([
        smartFeed === "today" ? getTodayArticles() : getUnreadArticles(),
        getSmartFeedCounts()
      ]);
      articles = smartArticles;
      smartFeedCounts = latestCounts;
      selectedArticle = null;
    } catch (err) {
      error = userError("Could not load smart feed articles.", err);
      articles = [];
    } finally {
      articlesLoading = false;
    }
  }

  async function refreshCurrentArticles() {
    if (viewKind === "tldr") {
      await loadTldr();
    } else if (viewKind === "feed" && selectedFeed) {
      await loadFeedArticles(selectedFeed.id);
    } else if (viewKind === "zone" && selectedZone) {
      await loadZoneArticles(selectedZone.id);
    } else if (viewKind === "smart" && selectedSmartFeed) {
      await loadSmartFeedArticles(selectedSmartFeed);
    }
  }

  async function handleAutoRefreshStatus(status: AutoRefreshStatus) {
    autoRefreshStatus = status;
    if (status.phase !== "completed" && status.phase !== "failed") return;

    await loadFeedsAndZones(false);
    if (selectedFeed) {
      selectedFeed = feeds.find((feed) => feed.id === selectedFeed?.id) ?? selectedFeed;
    }
    if (selectedZone) {
      selectedZone = zones.find((zone) => zone.id === selectedZone?.id) ?? selectedZone;
    }
    await refreshCurrentArticles();
    void refreshAiStatus(false);
  }

  function handleBehaviorUpdate(update: BehaviorUpdate) {
    const updatedZone = update.zone;
    if (!updatedZone) return;
    zones = zones.map((zone) => (zone.id === update.zone_id ? updatedZone : zone));
    if (selectedZone?.id === update.zone_id) selectedZone = updatedZone;
    if (zoneEditorZone?.id === update.zone_id) zoneEditorZone = updatedZone;
  }

  function isAutoRefreshRunning(): boolean {
    // The backlog drain is deliberately slow and runs for minutes. Showing it
    // as in-flight work would pin the progress hairline open the whole time,
    // so only a refresh cycle counts as running.
    if (autoRefreshStatus?.is_drain) return false;
    return autoRefreshStatus?.phase === "processing_zones";
  }

  async function refreshSelectedFeed() {
    if (!selectedFeed) return;
    await refreshSingleFeed(selectedFeed);
  }

  async function refreshSingleFeed(feed: Feed) {
    busyFeedId = feed.id;
    articlesLoading = true;
    error = null;
    try {
      await refreshFeed(feed.id);
      await loadFeedsAndZones(false);

      if (selectedFeed?.id === feed.id) {
        selectedFeed = feeds.find((updatedFeed) => updatedFeed.id === feed.id) ?? feed;
      }
      await refreshCurrentArticles();
    } catch (err) {
      error = userError("Could not refresh this source.", err);
    } finally {
      articlesLoading = false;
      busyFeedId = null;
    }
  }

  async function refreshAllFeeds() {
    if (feeds.length === 0) return;

    refreshingAll = true;
    articlesLoading = true;
    error = null;
    const failures: string[] = [];

    try {
      // Bounded concurrency: a serial pass over a large catalog can take
      // hours because every source carries its own request timeout.
      const queue = [...feeds];
      const refreshNext = async () => {
        for (;;) {
          const feed = queue.shift();
          if (!feed) return;
          try {
            await refreshFeed(feed.id);
          } catch (err) {
            failures.push(`${feed.title}: ${errorDetail(err)}`);
          }
        }
      };
      await Promise.all(
        Array.from({ length: Math.min(REFRESH_ALL_CONCURRENCY, queue.length) }, refreshNext)
      );

      await loadFeedsAndZones(false);
      if (selectedFeed) {
        selectedFeed =
          feeds.find((updatedFeed) => updatedFeed.id === selectedFeed?.id) ?? selectedFeed;
      }
      if (selectedZone) {
        selectedZone =
          zones.find((updatedZone) => updatedZone.id === selectedZone?.id) ?? selectedZone;
      }
      await refreshCurrentArticles();

      if (failures.length > 0) {
        error = userMessage("Some sources could not be refreshed.", failures.join("\n"));
      }
    } finally {
      busyFeedId = null;
      refreshingAll = false;
      articlesLoading = false;
    }
  }

  async function selectArticle(article: Article, url = article.url, context?: ReadContext) {
    const readContext = context ?? (
      viewKind === "zone" && selectedZone
        ? { kind: "zone" as const, zone_id: selectedZone.id }
        : viewKind === "smart"
          ? { kind: "smart_feed" as const }
          : { kind: "other" as const });
    const wasUnread = !article.is_read;
    selectedArticle = { ...article, is_read: true };
    if (wasUnread) {
      articles = articles.map((item) =>
        item.id === article.id ? { ...item, is_read: true } : item
      );
      if (viewKind === "smart" && selectedSmartFeed === "unread") {
        articles = articles.filter((item) => item.id !== article.id);
      }
    }
    void openArticle(url);
    void persistArticleOpen(article.id, readContext);
  }

  async function persistArticleOpen(articleId: number, readContext: ReadContext) {
    try {
      const result = await markRead(articleId, readContext);

      const unreadZoneIds = new Set(result.unread_zone_ids);
      const behaviorPending = new Map(
        result.behavior_progress.map((progress) => [progress.zone_id, progress.pending_reads])
      );
      zones = zones.map((zone) => {
        const pendingReads = behaviorPending.get(zone.id);
        if (!unreadZoneIds.has(zone.id) && pendingReads === undefined) return zone;
        return {
          ...zone,
          unread_count: unreadZoneIds.has(zone.id)
            ? Math.max(0, zone.unread_count - 1)
            : zone.unread_count,
          behavior_reads_pending: pendingReads ?? zone.behavior_reads_pending
        };
      });
      if (unreadZoneIds.size > 0) {
        smartFeedCounts = {
          ...smartFeedCounts,
          all_unread: Math.max(0, smartFeedCounts.all_unread - 1)
        };
      }
      if (selectedZone) {
        selectedZone = zones.find((zone) => zone.id === selectedZone?.id) ?? selectedZone;
      }
      if (zoneEditorZone) {
        zoneEditorZone = zones.find((zone) => zone.id === zoneEditorZone?.id) ?? zoneEditorZone;
      }
    } catch (err) {
      // The card was marked read optimistically, so resync from the database
      // rather than leaving the list showing state that was never saved.
      const openError = userError("Could not update read state.", err);
      await loadFeedsAndZones(false);
      await refreshCurrentArticles();
      error = openError;
    }
  }

  async function submitFeed(
    url: string,
    displayName: string | null,
    category: string,
    region: string | null
  ) {
    addFeedLoading = true;
    addFeedError = null;
    try {
      const savedFeed = await addFeed(url, displayName, category, region);
      await loadFeedsAndZones(false);
      const feed = feeds.find((item) => item.id === savedFeed.id) ?? savedFeed;
      addFeedOpen = false;
      if (viewKind === "sources") {
        openSources();
      } else {
        await selectFeed(feed);
      }
    } catch (err) {
      addFeedError = userError("Could not add this source.", err);
    } finally {
      addFeedLoading = false;
    }
  }

  function requestRemoveSource(feed: Feed) {
    deleteRequest = { kind: "source", item: feed };
  }

  async function removeSingleFeed(feed: Feed): Promise<boolean> {
    busyFeedId = feed.id;
    error = null;
    try {
      await removeFeed(feed.id);
      await loadFeedsAndZones(false);

      if (selectedFeed?.id === feed.id) {
        selectedFeed = null;
        selectedArticle = null;
        articles = [];
        if (zones.length > 0) {
          await selectZone(zones[0]);
        } else {
          openSources();
        }
      } else if (selectedZone) {
        await loadZoneArticles(selectedZone.id);
      } else if (selectedSmartFeed) {
        await loadSmartFeedArticles(selectedSmartFeed);
      }
      return true;
    } catch (err) {
      error = userError("Could not remove this source.", err);
      return false;
    } finally {
      busyFeedId = null;
    }
  }

  function openCreateZone() {
    zoneEditorZone = null;
    zoneEditorFeedIds = [];
    zoneEditorError = null;
    zoneEditorOpen = true;
  }

  async function openEditZone(zone: Zone) {
    zoneEditorLoading = true;
    zoneEditorError = null;
    try {
      const assignedFeeds = await getZoneFeeds(zone.id);
      zoneEditorZone = zone;
      zoneEditorFeedIds = assignedFeeds.map((feed) => feed.id);
      zoneEditorOpen = true;
    } catch (err) {
      error = userError("Could not load Xone settings.", err);
    } finally {
      zoneEditorLoading = false;
    }
  }

  async function submitZone(
    name: string,
    filterDescription: string,
    focusLevel: Zone["focus_level"],
    feedIds: number[]
  ) {
    if (feedIds.length > SOURCE_LIMIT) {
      zoneEditorError = userMessage(`Choose ${SOURCE_LIMIT} sources or fewer.`);
      return;
    }

    zoneEditorLoading = true;
    zoneEditorError = null;
    try {
      const savedZone = zoneEditorZone
        ? await updateZone(zoneEditorZone.id, name, filterDescription, focusLevel)
        : await createZone(name, filterDescription, focusLevel);
      const currentFeeds = zoneEditorZone ? await getZoneFeeds(savedZone.id) : [];
      const currentIds = new Set(currentFeeds.map((feed) => feed.id));
      const nextIds = new Set(feedIds);

      for (const feedId of nextIds) {
        if (!currentIds.has(feedId)) await assignFeed(savedZone.id, feedId);
      }
      for (const feedId of currentIds) {
        if (!nextIds.has(feedId)) await unassignFeed(savedZone.id, feedId);
      }

      await loadFeedsAndZones(false);
      const zone = zones.find((item) => item.id === savedZone.id) ?? savedZone;
      zoneEditorOpen = false;
      await selectZone(zone);
    } catch (err) {
      zoneEditorError = userError("Could not save this Xone.", err);
    } finally {
      zoneEditorLoading = false;
    }
  }

  function requestRemoveZone(zone: Zone) {
    deleteRequest = { kind: "zone", item: zone };
  }

  async function retryBehaviorLearning(zone: Zone) {
    zoneEditorError = null;
    try {
      await retryZoneBehavior(zone.id);
    } catch (err) {
      zoneEditorError = userError("Could not retry behavior learning.", err);
    }
  }

  function requestResetBehavior(zone: Zone) {
    deleteRequest = { kind: "behavior", item: zone };
  }

  async function resetBehaviorLearning(zone: Zone): Promise<boolean> {
    zoneEditorError = null;
    try {
      const resetZone = await resetZoneBehavior(zone.id);
      zones = zones.map((item) => (item.id === zone.id ? resetZone : item));
      zoneEditorZone = resetZone;
      if (selectedZone?.id === zone.id) selectedZone = zoneEditorZone;
      return true;
    } catch (err) {
      zoneEditorError = userError("Could not reset learned behavior.", err);
      return false;
    }
  }

  async function removeZone(zone: Zone): Promise<boolean> {
    error = null;
    try {
      await deleteZone(zone.id);
      await loadFeedsAndZones(false);

      if (selectedZone?.id === zone.id) {
        selectedZone = null;
        selectedArticle = null;
        articles = [];
        if (zones.length > 0) {
          await selectZone(zones[0]);
        } else {
          openSources();
        }
      }
      return true;
    } catch (err) {
      error = userError("Could not delete this Xone.", err);
      return false;
    }
  }

  async function confirmDelete() {
    if (!deleteRequest || deleteLoading) return;

    deleteLoading = true;
    try {
      // Keep the dialog open when the action failed, so the error is shown
      // next to the item it belongs to.
      const completed =
        deleteRequest.kind === "source"
          ? await removeSingleFeed(deleteRequest.item)
          : deleteRequest.kind === "behavior"
            ? await resetBehaviorLearning(deleteRequest.item)
            : await removeZone(deleteRequest.item);
      if (completed) deleteRequest = null;
    } finally {
      deleteLoading = false;
    }
  }

  function cancelDelete() {
    if (!deleteLoading) deleteRequest = null;
  }

  function deleteDialogTitle(): string {
    if (deleteRequest?.kind === "source") return "Remove Source";
    if (deleteRequest?.kind === "behavior") return "Reset Learned Behavior";
    return "Delete Xone";
  }

  function deleteDialogMessage(): string {
    if (deleteRequest?.kind === "source") {
      return `Remove source "${deleteRequest.item.title}" from Xone?`;
    }
    if (deleteRequest?.kind === "zone") {
      return `Delete Xone "${deleteRequest.item.name}"? Sources and articles will stay in Xone.`;
    }
    if (deleteRequest?.kind === "behavior") {
      return `Reset learned behavior for "${deleteRequest.item.name}"? Its Signal brief, articles, classifications, and global read state will stay unchanged.`;
    }
    return "";
  }

  async function openArticle(url: string) {
    try {
      await openInBrowser(url);
    } catch (err) {
      error = userError("Could not open article.", err);
    }
  }

  function openSettings() {
    settingsOpen = true;
    settingsError = null;
  }

  function closeSettings() {
    renderAppearance(appearance);
    settingsOpen = false;
  }

  async function submitSettings(
    settings: AiSettings,
    nextAppSettings: AppSettings,
    nextAppearance: AppearanceSettings
  ) {
    settingsOpen = false;
    settingsError = null;
    applyAppearance(nextAppearance);
    try {
      aiSettings = await updateAiSettings(settings);
    } catch (err) {
      settingsError = userError("Could not save AI provider settings.", err);
      settingsOpen = true;
      return;
    }

    try {
      appSettings = await updateAppSettings(nextAppSettings);
    } catch (err) {
      settingsError = userError(
        "AI provider settings were saved, but the refresh and cleanup settings were not.",
        err
      );
      settingsOpen = true;
    }
  }

  async function deleteOldArticlesNow(retentionDays: ArticleRetentionDays): Promise<number> {
    const deleted = await cleanupOldArticles(retentionDays);
    await loadFeedsAndZones(false);
    if (selectedFeed) {
      selectedFeed = feeds.find((feed) => feed.id === selectedFeed?.id) ?? selectedFeed;
    }
    if (selectedZone) {
      selectedZone = zones.find((zone) => zone.id === selectedZone?.id) ?? selectedZone;
    }
    await refreshCurrentArticles();
    return deleted;
  }

  async function refreshAiStatus(showErrors = true) {
    try {
      aiStatus = await getAiStatus();
    } catch (err) {
      const statusError = userError("AI status check failed.", err);
      aiStatus = { connected: false, provider: "unknown", message: statusError.message };
      if (showErrors) settingsError = statusError;
    }
  }

  async function loadAiModels(settings: AiSettings): Promise<AiModel[]> {
    return fetchAiModels(settings);
  }

  async function classifySelectedZone(reclassify: boolean) {
    if (!selectedZone || classifyingZone) return;

    const zoneId = selectedZone.id;
    classifyingZone = true;
    articlesLoading = true;
    error = null;
    reclassifyResult = null;
    try {
      const processingLimit =
        appSettings?.classification_processing_mode === "limited"
          ? appSettings.classification_processing_limit
          : ALL_UNCLASSIFIED;
      reclassifyResult = await classifyZone(zoneId, reclassify, processingLimit);
      await loadFeedsAndZones(false);
      selectedZone = zones.find((zone) => zone.id === zoneId) ?? selectedZone;
      await loadZoneArticles(zoneId);
      await refreshAiStatus(false);
    } catch (err) {
      error = userError("Could not refresh and classify this Xone.", err);
    } finally {
      classifyingZone = false;
      articlesLoading = false;
    }
  }

  // Focus is a stored re-filter of matches the model already made, so the
  // list can be re-read straight from SQLite without another model call.
  async function setZoneFocus(level: FocusLevel) {
    const zone = selectedZone;
    if (!zone || focusSaving || zone.focus_level === level) return;
    focusSaving = true;
    error = null;
    try {
      const updated = await updateZone(zone.id, zone.name, zone.filter_description, level);
      zones = zones.map((item) => (item.id === updated.id ? updated : item));
      selectedZone = updated;
      const [, counts] = await Promise.all([loadZoneArticles(updated.id), getSmartFeedCounts()]);
      smartFeedCounts = counts;
    } catch (err) {
      error = userError("Could not change the focus level.", err);
    } finally {
      focusSaving = false;
    }
  }

  function handleAppShortcut(event: KeyboardEvent) {
    if (!event.metaKey && !event.ctrlKey) return;
    if (document.querySelector(".modal-backdrop")) return;

    if (event.key.toLowerCase() === "r") {
      event.preventDefault();
      if (viewKind === "tldr") void rebuildTldr();
      else if (viewKind === "zone" && selectedZone) void classifySelectedZone(false);
      else if (viewKind === "feed" && selectedFeed) void refreshSelectedFeed();
      else void refreshAllFeeds();
      return;
    }

    const position = Number(event.key);
    if (Number.isInteger(position) && position >= 1 && position <= 9) {
      const zone = zones[position - 1];
      if (!zone) return;
      event.preventDefault();
      void selectZone(zone);
    }
  }

  function listLabel(): string {
    if (viewKind === "zone" && selectedZone) return "Xone";
    if (viewKind === "feed" && selectedFeed) return "Source";
    if (viewKind === "smart") return "Smart Feed";
    return "Articles";
  }

  function listTitle(): string {
    if (viewKind === "zone" && selectedZone) return selectedZone.name;
    if (viewKind === "feed" && selectedFeed) return selectedFeed.title;
    if (viewKind === "smart") {
      return selectedSmartFeed === "unread" ? "All Unread" : "Today";
    }
    return "Your briefing starts here";
  }

  function emptyText(): string {
    if (viewKind === "zone" && !selectedZone) return "Choose a Xone or create one to begin.";
    if (viewKind === "feed" && !selectedFeed) return "Choose a Xone or create one to begin.";
    if (viewKind === "zone" && selectedZoneFeedCount === 0) return "Add sources to this Xone.";
    if (viewKind === "zone") return "No articles match this filter.";
    if (viewKind === "feed") return "No stored articles for this source yet.";
    if (viewKind === "smart" && selectedSmartFeed === "unread") {
      return "You're all caught up.";
    }
    if (viewKind === "smart") return "No articles published today.";
    return "Choose a Xone or create one to begin.";
  }

  function errorDetail(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  function userMessage(message: string, detail?: string): UserError {
    const normalizedDetail = detail?.replace(/\s+/g, " ").trim();
    return normalizedDetail ? { message, detail: normalizedDetail } : { message };
  }

  function userError(message: string, err: unknown): UserError {
    const detail = errorDetail(err).replace(/\s+/g, " ").trim();
    console.error(message, err);
    return userMessage(message, detail);
  }
</script>

<svelte:window onkeydown={handleAppShortcut} />

<main class="app-shell" style={`--sidebar-width: ${sidebarWidth}px`}>
  <!-- The window has no title bar of its own, so this strip is what it gets
       dragged by. It covers only the inset reserved for the traffic lights. -->
  <div class="titlebar-drag" data-tauri-drag-region></div>
  <Sidebar
    {zones}
    {feeds}
    {smartFeedCounts}
    activeSmartFeed={viewKind === "smart" || viewKind === "tldr" ? selectedSmartFeed : null}
    selectedZoneId={selectedZone?.id ?? null}
    activeSection={viewKind === "sources" ? "sources" : "articles"}
    loading={feedsLoading}
    onSelectZone={selectZone}
    onSelectSmartFeed={selectSmartFeed}
    onCreateZone={openCreateZone}
    onEditZone={openEditZone}
    onDeleteZone={requestRemoveZone}
    onReorderZones={saveZoneOrder}
    onOpenSources={openSources}
    onOpenSettings={openSettings}
    onToggleTheme={toggleTheme}
    {theme}
    {aiStatus}
  />
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="sidebar-resizer"
    role="separator"
    aria-label="Resize sidebar"
    aria-orientation="vertical"
    aria-valuemin={MIN_SIDEBAR_WIDTH}
    aria-valuemax={MAX_SIDEBAR_WIDTH}
    aria-valuenow={sidebarWidth}
    tabindex="0"
    title="Drag to resize sidebar"
    onpointerdown={startSidebarResize}
    onpointermove={resizeSidebar}
    onpointerup={stopSidebarResize}
    onpointercancel={stopSidebarResize}
    onkeydown={resizeSidebarWithKeyboard}
  ></div>
  <section class="content-pane" id="main-content">
    {#if viewKind === "sources"}
      <SourcesPage
        {feeds}
        loading={feedsLoading}
        {busyFeedId}
        refreshingAll={refreshingAll || isAutoRefreshRunning()}
        {error}
        {autoRefreshStatus}
        onDismissAutoRefresh={() => (autoRefreshStatus = null)}
        onAddSource={() => {
          addFeedError = null;
          addFeedOpen = true;
        }}
        onRefreshAll={refreshAllFeeds}
        onRefreshSource={refreshSingleFeed}
        onRemoveSource={requestRemoveSource}
        onViewSource={selectFeed}
      />
    {:else if viewKind === "tldr"}
      <TldrPage
        {zones}
        entries={tldrEntries}
        loading={tldrLoading}
        error={tldrError}
        onReload={loadTldr}
        onRegenerate={rebuildTldr}
        onSelectZone={selectZone}
        onOpenCitation={(article, zoneId) => selectArticle(article, article.url, { kind: "zone", zone_id: zoneId })}
        onCreateZone={openCreateZone}
        onOpenSettings={openSettings}
      />
    {:else}
      <ArticleList
        title={listTitle()}
        label={listLabel()}
        lastUpdatedAt={viewKind === "zone" && selectedZone
          ? selectedZone.last_updated_at
          : undefined}
        emptyText={emptyText()}
        {articles}
        showSourceName={viewKind === "zone" || viewKind === "smart"}
        focusLevel={viewKind === "zone" && selectedZone ? selectedZone.focus_level : null}
        {focusSaving}
        onSetFocus={viewKind === "zone" && selectedZone ? setZoneFocus : null}
        {sourceNames}
        selectedArticleId={selectedArticle?.id ?? null}
        loading={articlesLoading || isAutoRefreshRunning()}
        listBusy={articlesLoading || focusSaving}
        {error}
        onSelect={selectArticle}
        onRefresh={viewKind === "feed" ? refreshSelectedFeed : null}
        onReclassify={viewKind === "zone" ? () => classifySelectedZone(false) : null}
        onDismissReclassifyResult={() => (reclassifyResult = null)}
        reclassifyLoading={classifyingZone}
        {reclassifyResult}
        {autoRefreshStatus}
        onDismissAutoRefresh={() => (autoRefreshStatus = null)}
      />
    {/if}
  </section>
  <AddFeed
    open={addFeedOpen}
    loading={addFeedLoading}
    error={addFeedError}
    {feeds}
    onSubmit={submitFeed}
    onClose={() => {
      if (!addFeedLoading) addFeedOpen = false;
    }}
  />
  <ZoneEditor
    open={zoneEditorOpen}
    loading={zoneEditorLoading}
    error={zoneEditorError}
    zone={zoneEditorZone}
    {feeds}
    assignedFeedIds={zoneEditorFeedIds}
    sourceLimit={SOURCE_LIMIT}
    onAssistFilter={assistFilterBrief}
    onRecommend={recommendSources}
    onRetryBehavior={retryBehaviorLearning}
    onResetBehavior={requestResetBehavior}
    onSubmit={submitZone}
    onClose={() => {
      if (!zoneEditorLoading) zoneEditorOpen = false;
    }}
  />
  <Settings
    open={settingsOpen}
    error={settingsError}
    settings={aiSettings}
    {appSettings}
    {appearance}
    onSubmit={submitSettings}
    onPreviewAppearance={previewAppearance}
    onFetchModels={loadAiModels}
    onCountOldArticles={getArticleCleanupCount}
    onDeleteOldArticles={deleteOldArticlesNow}
    onClose={closeSettings}
  />
  <ConfirmDialog
    open={deleteRequest !== null}
    title={deleteDialogTitle()}
    message={deleteDialogMessage()}
    confirmLabel={deleteRequest?.kind === "source"
      ? "Remove Source"
      : deleteRequest?.kind === "behavior"
        ? "Reset Behavior"
        : "Delete Xone"}
    loading={deleteLoading}
    onConfirm={confirmDelete}
    onCancel={cancelDelete}
  />
</main>
