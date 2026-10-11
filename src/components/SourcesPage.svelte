<script lang="ts">
  import { ArrowSquareOut, ArrowsClockwise, Plus, Trash } from "phosphor-svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import RunDetails from "./RunDetails.svelte";
  import type { AutoRefreshStatus, Feed, UserError } from "../lib/types";

  interface Props {
    feeds: Feed[];
    loading: boolean;
    busyFeedId: number | null;
    refreshingAll: boolean;
    error: UserError | null;
    autoRefreshStatus: AutoRefreshStatus | null;
    onDismissAutoRefresh: () => void;
    onAddSource: () => void;
    onRefreshAll: () => void;
    onRefreshSource: (feed: Feed) => void;
    onRemoveSource: (feed: Feed) => void;
    onViewSource: (feed: Feed) => void;
  }

  let {
    feeds,
    loading,
    busyFeedId,
    refreshingAll,
    error,
    autoRefreshStatus,
    onDismissAutoRefresh,
    onAddSource,
    onRefreshAll,
    onRefreshSource,
    onRemoveSource,
    onViewSource
  }: Props = $props();

  type SourceGroup = [string, Feed[]];
  let sourceQuery = $state("");

  function matchesSource(feed: Feed): boolean {
    const query = sourceQuery.trim().toLocaleLowerCase();
    if (!query) return true;
    return [
      feed.title,
      feed.description,
      feed.site_url,
      sourceLocation(feed),
      feed.category,
      feed.region
    ]
      .filter((value): value is string => !!value)
      .some((value) => value.toLocaleLowerCase().includes(query));
  }

  function groupSources(groupBy: (feed: Feed) => string | null): SourceGroup[] {
    const groups = new Map<string, Feed[]>();
    for (const feed of feeds) {
      if (!matchesSource(feed)) continue;
      const group = groupBy(feed);
      if (!group) continue;
      groups.set(group, [...(groups.get(group) ?? []), feed]);
    }
    return Array.from(groups.entries()).sort(([left], [right]) => {
      if (left === "World") return -1;
      if (right === "World") return 1;
      return left.localeCompare(right);
    });
  }

  function regionalSources(): SourceGroup[] {
    return groupSources((feed) => feed.region?.trim() || null);
  }

  function topicalSources(): SourceGroup[] {
    return groupSources((feed) =>
      feed.region?.trim() ? null : feed.category?.trim() || "Custom Sources"
    );
  }

  function shouldOpenGroup(): boolean {
    return !!sourceQuery.trim();
  }

  function formatDate(value: string | null): string {
    if (!value) return "Never fetched";
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit"
    }).format(new Date(value));
  }

  function sourceLocation(feed: Feed): string {
    try {
      const url = new URL(feed.url);
      if (url.hostname !== "news.google.com" || url.pathname !== "/rss/search") {
        return feed.url;
      }

      const query = url.searchParams.get("q") ?? "";
      const publisher = query.match(/(?:^|\s|\()site:([^\s)]+)/i)?.[1];
      return publisher ?? "Google News";
    } catch {
      return feed.url;
    }
  }
</script>

{#snippet sourceRows(sourceFeeds: Feed[])}
  <div class="source-table">
    {#each sourceFeeds as feed}
      <article class="source-row">
        <div class="source-main">
          <h4>{feed.title}</h4>
          <p>{sourceLocation(feed)}</p>
        </div>
        <div class="source-meta">
          <span>{formatDate(feed.last_fetched_at)}</span>
        </div>
        <div class="source-row-actions">
          <button
            class="icon-button source-icon-button"
            type="button"
            onclick={(event) => {
              event.stopPropagation();
              onViewSource(feed);
            }}
            aria-label={`View ${feed.title}`}
            title="View source articles"
          >
            <ArrowSquareOut size={16} weight="regular" aria-hidden="true" />
          </button>
          <button
            class="icon-button source-icon-button"
            type="button"
            onclick={(event) => {
              event.stopPropagation();
              onRefreshSource(feed);
            }}
            disabled={busyFeedId === feed.id || refreshingAll}
            aria-label={busyFeedId === feed.id
              ? `Refreshing ${feed.title}`
              : `Refresh ${feed.title}`}
            title={busyFeedId === feed.id ? "Refreshing source" : "Refresh source"}
          >
            <ArrowsClockwise size={16} weight="regular" aria-hidden="true" />
          </button>
          <button
            class="icon-button source-icon-button danger-button"
            type="button"
            onclick={(event) => {
              event.stopPropagation();
              onRemoveSource(feed);
            }}
            disabled={busyFeedId === feed.id || refreshingAll}
            aria-label={`Remove ${feed.title}`}
            title="Remove source"
          >
            <Trash size={16} weight="regular" aria-hidden="true" />
          </button>
        </div>
      </article>
    {/each}
  </div>
{/snippet}

<section class="sources-page" aria-label="Sources">
  <header class="toolbar" data-tauri-drag-region>
    <div class="toolbar-title">
      <h2>Sources</h2>
      <p class="toolbar-sub">
        <span>{feeds.length} configured</span>
      </p>
    </div>
    <div class="sources-actions">
      <button
        class="icon-button source-icon-button"
        type="button"
        onclick={onAddSource}
        disabled={loading || refreshingAll}
        aria-label="Add source"
        title="Add source"
      >
        <Plus size={17} weight="regular" aria-hidden="true" />
      </button>
      <button
        class="icon-button source-icon-button"
        type="button"
        onclick={onRefreshAll}
        disabled={loading || refreshingAll || feeds.length === 0}
        aria-label={refreshingAll ? "Refreshing all sources" : "Refresh all sources"}
        title={refreshingAll ? "Refreshing all sources" : "Refresh all sources"}
      >
        <ArrowsClockwise size={17} weight="regular" aria-hidden="true" />
      </button>
      <RunDetails
        {autoRefreshStatus}
        {onDismissAutoRefresh}
        reclassifyResult={null}
        onDismissReclassifyResult={() => {}}
      />
    </div>

    {#if refreshingAll || autoRefreshStatus?.phase === "processing_zones"}
      <div class="toolbar-progress" role="status" aria-label="Working"></div>
    {/if}
  </header>

  <ErrorNotice {error} />

  {#if loading}
    <p class="empty">Loading sources...</p>
  {:else if feeds.length === 0}
    <p class="empty">Add a source to start collecting articles.</p>
  {:else}
    <div class="source-catalog-search">
      <input
        type="search"
        bind:value={sourceQuery}
        placeholder="Search sources"
        aria-label="Search source catalog"
      />
    </div>
    <div class="source-groups">
      {#if regionalSources().length > 0}
        <section class="source-region">
          <div class="source-region-heading">
            <h3>Regions</h3>
            <span>{regionalSources().reduce((count, [, group]) => count + group.length, 0)} sources</span>
          </div>

          <div class="source-region-groups">
            {#each regionalSources() as [region, regionFeeds]}
              <details class="source-group" open={shouldOpenGroup()}>
                <summary class="source-group-heading">
                  <h4>{region}</h4>
                  <span>{regionFeeds.length} sources</span>
                </summary>

                {@render sourceRows(regionFeeds)}
              </details>
            {/each}
          </div>
        </section>
      {/if}

      {#if topicalSources().length > 0}
        <section class="source-region">
          <div class="source-region-heading">
            <h3>Topics</h3>
            <span>{topicalSources().reduce((count, [, group]) => count + group.length, 0)} sources</span>
          </div>

          <div class="source-region-groups">
            {#each topicalSources() as [category, categoryFeeds]}
              <details class="source-group" open={shouldOpenGroup()}>
                <summary class="source-group-heading">
                  <h4>{category}</h4>
                  <span>{categoryFeeds.length} sources</span>
                </summary>

                {@render sourceRows(categoryFeeds)}
              </details>
            {/each}
          </div>
        </section>
      {/if}

      {#if regionalSources().length === 0 && topicalSources().length === 0}
        <p class="empty compact">No sources match “{sourceQuery.trim()}”.</p>
      {/if}
    </div>
  {/if}
</section>
