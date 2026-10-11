<script lang="ts">
  import {
    ArrowsClockwise,
    CaretDown,
    CaretRight,
    MagnifyingGlass,
    StackSimple,
    X
  } from "phosphor-svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import RunDetails from "./RunDetails.svelte";
  import type {
    Article,
    AutoRefreshStatus,
    ClassifyZoneResult,
    FocusLevel,
    MatchLevel,
    UserError
  } from "../lib/types";

  interface Props {
    title: string;
    label: string;
    lastUpdatedAt: string | null | undefined;
    emptyText: string;
    articles: Article[];
    showSourceName: boolean;
    sourceNames: ReadonlyMap<number, string>;
    selectedArticleId: number | null;
    loading: boolean;
    listBusy: boolean;
    error: UserError | null;
    focusLevel: FocusLevel | null;
    focusSaving: boolean;
    onSetFocus: ((level: FocusLevel) => void) | null;
    onSelect: (article: Article, url?: string) => void;
    onRefresh: (() => void) | null;
    onReclassify: (() => void) | null;
    onDismissReclassifyResult: () => void;
    reclassifyLoading: boolean;
    reclassifyResult: ClassifyZoneResult | null;
    autoRefreshStatus: AutoRefreshStatus | null;
    onDismissAutoRefresh: () => void;
  }

  let {
    title,
    label,
    lastUpdatedAt,
    emptyText,
    articles,
    showSourceName,
    sourceNames,
    selectedArticleId,
    loading,
    listBusy,
    error,
    focusLevel,
    focusSaving,
    onSetFocus,
    onSelect,
    onRefresh,
    onReclassify,
    onDismissReclassifyResult,
    reclassifyLoading,
    reclassifyResult,
    autoRefreshStatus,
    onDismissAutoRefresh
  }: Props = $props();

  const FOCUS_OPTIONS: Array<{ value: FocusLevel; label: string; hint: string }> = [
    { value: "broad", label: "Broad", hint: "Show low, medium, and high matches" },
    { value: "focused", label: "Focused", hint: "Show medium and high matches" },
    { value: "laser", label: "Laser", hint: "Show high matches only" }
  ];

  let searchQuery = $state("");
  let searchOpen = $state(false);
  let expandedArticleIds = $state<Set<number>>(new Set());
  let cursorIndex = $state(-1);
  let searchInput = $state<HTMLInputElement | null>(null);
  let listElement = $state<HTMLElement | null>(null);

  let visibleArticles = $derived.by(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) return articles;
    return articles.filter((article) => {
      const searchable = [
        article.title,
        article.summary ?? "",
        article.content,
        article.author ?? "",
        ...article.related_articles.flatMap((related) => [related.title, related.summary ?? ""])
      ]
        .join(" ")
        .toLowerCase();
      return searchable.includes(query);
    });
  });

  // Rows carry their own day band so the list stays one flat scroll container
  // and the sticky headers hand off to each other cleanly.
  let rows = $derived.by(() => {
    let previousBand = "";
    return visibleArticles.map((article, index) => {
      const band = dayBand(article.published_at ?? article.fetched_at);
      const startsBand = band !== previousBand;
      previousBand = band;
      return { article, index, band, startsBand };
    });
  });

  function dayBand(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return "Undated";
    const startOfDay = (input: Date) =>
      new Date(input.getFullYear(), input.getMonth(), input.getDate()).getTime();
    const dayGap = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86_400_000);
    if (dayGap === 0) return "Today";
    if (dayGap === 1) return "Yesterday";
    if (dayGap > 1 && dayGap < 7) {
      return new Intl.DateTimeFormat(undefined, { weekday: "long" }).format(date);
    }
    return new Intl.DateTimeFormat(undefined, { month: "long", day: "numeric" }).format(date);
  }

  function formatTime(value: string | null): string {
    if (!value) return "No date";
    return new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(
      new Date(value)
    );
  }

  function formatStamp(value: string): string {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit"
    }).format(new Date(value));
  }

  function plural(count: number, label: string): string {
    return `${count} ${label}${count === 1 ? "" : "s"}`;
  }

  function matchLabel(level: MatchLevel | null): string | null {
    if (!level || level === "no") return null;
    return `${level.charAt(0).toUpperCase()}${level.slice(1)} match`;
  }

  function displaySummary(value: string | null): string | null {
    const summary = value?.replace(/\s+/g, " ").trim();
    if (!summary) return null;
    if (/^https?:\/\//i.test(summary)) return null;
    if (/\b(Article|Comments) URL:\s*https?:\/\//i.test(summary)) return null;
    return summary;
  }

  function toggleRelatedArticles(articleId: number): void {
    const next = new Set(expandedArticleIds);
    if (next.has(articleId)) next.delete(articleId);
    else next.add(articleId);
    expandedArticleIds = next;
  }

  function openSearch(): void {
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }

  function closeSearch(): void {
    searchQuery = "";
    searchOpen = false;
  }

  function moveCursor(delta: number): void {
    if (visibleArticles.length === 0) return;
    const next = Math.max(0, Math.min(visibleArticles.length - 1, cursorIndex + delta));
    cursorIndex = cursorIndex < 0 ? 0 : next;
    listElement
      ?.querySelector<HTMLElement>(`[data-row-index="${cursorIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }

  function handleShortcut(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    const typing =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement ||
      target?.isContentEditable === true;
    if (document.querySelector(".modal-backdrop")) return;

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      openSearch();
      return;
    }
    if (event.key === "Escape" && searchOpen) {
      event.preventDefault();
      closeSearch();
      return;
    }
    if (typing || event.metaKey || event.ctrlKey || event.altKey) return;

    if (event.key === "j" || event.key === "ArrowDown") {
      event.preventDefault();
      moveCursor(1);
    } else if (event.key === "k" || event.key === "ArrowUp") {
      event.preventDefault();
      moveCursor(-1);
    } else if (event.key === "Enter" || event.key === "o") {
      const article = visibleArticles[cursorIndex];
      if (!article) return;
      event.preventDefault();
      onSelect(article);
    } else if (event.key === "/") {
      event.preventDefault();
      openSearch();
    }
  }
</script>

<svelte:window onkeydown={handleShortcut} />

<section class="article-list" aria-label="Articles">
  <header class="toolbar" data-tauri-drag-region>
    <div class="toolbar-title">
      <h2>{title}</h2>
      <p class="toolbar-sub">
        <span>{label}</span>
        {#if articles.length > 0}
          <span>{plural(articles.length, "article")}</span>
        {/if}
        {#if lastUpdatedAt !== undefined}
          <span>
            {#if lastUpdatedAt}
              Updated <time datetime={lastUpdatedAt}>{formatStamp(lastUpdatedAt)}</time>
            {:else}
              Never updated
            {/if}
          </span>
        {/if}
      </p>
    </div>

    <div class="toolbar-actions">
      {#if onSetFocus && focusLevel}
        <div class="focus-dial" role="group" aria-label="Xone focus">
          {#each FOCUS_OPTIONS as option}
            <button
              class="focus-segment"
              class:active={focusLevel === option.value}
              type="button"
              disabled={focusSaving || focusLevel === option.value}
              aria-pressed={focusLevel === option.value}
              title={option.hint}
              onclick={() => onSetFocus?.(option.value)}
            >
              {option.label}
            </button>
          {/each}
        </div>
      {/if}

      {#if searchOpen}
        <div class="toolbar-search">
          <MagnifyingGlass size={14} weight="regular" aria-hidden="true" />
          <!-- svelte-ignore a11y_autofocus -->
          <input
            bind:this={searchInput}
            bind:value={searchQuery}
            type="search"
            placeholder="Search these articles"
            aria-label="Search these articles"
            onblur={() => {
              if (!searchQuery) searchOpen = false;
            }}
          />
          <button
            class="icon-button toolbar-icon-button"
            type="button"
            onclick={closeSearch}
            aria-label="Close search"
            title="Close search"
          >
            <X size={13} weight="regular" aria-hidden="true" />
          </button>
        </div>
      {:else}
        <button
          class="icon-button toolbar-icon-button"
          type="button"
          onclick={openSearch}
          aria-label="Search these articles"
          title="Search these articles (⌘F)"
        >
          <MagnifyingGlass size={16} weight="regular" aria-hidden="true" />
        </button>
      {/if}

      {#if onRefresh}
        <button
          class="icon-button toolbar-icon-button"
          type="button"
          onclick={onRefresh}
          disabled={loading}
          aria-label={loading ? "Fetching new articles" : "Fetch new articles"}
          title={loading ? "Fetching new articles" : "Fetch new articles (⌘R)"}
        >
          <ArrowsClockwise size={16} weight="regular" aria-hidden="true" />
        </button>
      {/if}

      {#if onReclassify}
        <button
          class="icon-button toolbar-icon-button"
          type="button"
          onclick={onReclassify}
          disabled={loading || reclassifyLoading}
          aria-label={reclassifyLoading ? "Fetching and matching" : "Fetch and match new articles"}
          title={reclassifyLoading
            ? "Fetching and matching"
            : "Fetch new articles and match them to this Xone (⌘R)"}
        >
          <ArrowsClockwise size={16} weight="regular" aria-hidden="true" />
        </button>
      {/if}

      <RunDetails
        {autoRefreshStatus}
        {reclassifyResult}
        {onDismissAutoRefresh}
        {onDismissReclassifyResult}
      />
    </div>

    {#if loading || reclassifyLoading || listBusy}
      <div class="toolbar-progress" role="status" aria-label="Working"></div>
    {/if}
  </header>

  <div class="list-body" bind:this={listElement} aria-busy={listBusy}>
    <ErrorNotice {error} />

    {#if (loading || listBusy) && articles.length === 0}
      <p class="empty">{listBusy ? "Loading articles" : "Fetching recent articles"}</p>
    {:else if articles.length === 0}
      <p class="empty">{emptyText}</p>
    {:else if visibleArticles.length === 0}
      <p class="empty">Nothing here matches “{searchQuery.trim()}”. Try fewer words.</p>
    {:else}
      <div class="article-rows">
        {#each rows as row (row.article.id)}
          {@const article = row.article}
          {@const summary = displaySummary(article.summary)}
          {@const hasRelated = article.related_articles.length > 0}
          {@const expanded = expandedArticleIds.has(article.id)}
          {#if row.startsBand}
            <h3 class="day-band">{row.band}</h3>
          {/if}
          <div
            class="article-row"
            class:cursor={row.index === cursorIndex}
            class:active={article.id === selectedArticleId}
            data-row-index={row.index}
          >
            <span class="read-state" aria-hidden="true">
              {#if !article.is_read}<span class="unread-dot"></span>{/if}
            </span>
            <button class="article-open" type="button" onclick={() => onSelect(article)}>
              <span class="article-title">{article.title}</span>
              {#if summary}
                <span class="article-summary">{summary}</span>
              {/if}
              <span class="article-meta">
                <span>{formatTime(article.published_at ?? article.fetched_at)}</span>
                {#if showSourceName && sourceNames.get(article.feed_id)}
                  <span class="article-source">{sourceNames.get(article.feed_id)}</span>
                {/if}
                {#if matchLabel(article.ai_match_level)}
                  <span class="match-label" data-match={article.ai_match_level}>
                    {matchLabel(article.ai_match_level)}
                  </span>
                {/if}
              </span>
              {#if !article.is_read}
                <span class="visually-hidden">Unread</span>
              {/if}
            </button>
            <div class="row-side">
              {#if hasRelated}
              <button
                class="cluster-mark"
                type="button"
                onclick={() => toggleRelatedArticles(article.id)}
                aria-expanded={expanded}
                aria-controls={`related-articles-${article.id}`}
                aria-label={`${expanded ? "Hide" : "Show"} ${plural(article.related_articles.length, "other report")}`}
                title={`${plural(article.related_articles.length + 1, "report")} of this story`}
              >
                <StackSimple size={13} weight="regular" aria-hidden="true" />
                <span>{article.related_articles.length + 1}</span>
                {#if expanded}
                  <CaretDown size={9} weight="bold" aria-hidden="true" />
                {:else}
                  <CaretRight size={9} weight="bold" aria-hidden="true" />
                {/if}
              </button>
              {/if}
            </div>
          </div>
          {#if hasRelated && expanded}
            <div class="related-articles" id={`related-articles-${article.id}`}>
              {#each article.related_articles as related}
                <button
                  class="related-article"
                  type="button"
                  onclick={() => onSelect(article, related.url)}
                >
                  <span class="article-title">{related.title}</span>
                  <span class="article-meta">
                    <span>{formatTime(related.published_at ?? related.fetched_at)}</span>
                    {#if sourceNames.get(related.feed_id)}
                      <span class="article-source">{sourceNames.get(related.feed_id)}</span>
                    {/if}
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </div>
</section>
