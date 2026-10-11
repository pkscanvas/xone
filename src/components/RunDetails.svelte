<script lang="ts">
  import { Info } from "phosphor-svelte";
  import type { AutoRefreshStatus, ClassifyZoneResult } from "../lib/types";

  interface Props {
    autoRefreshStatus: AutoRefreshStatus | null;
    reclassifyResult: ClassifyZoneResult | null;
    onDismissAutoRefresh: () => void;
    onDismissReclassifyResult: () => void;
  }

  let {
    autoRefreshStatus,
    reclassifyResult,
    onDismissAutoRefresh,
    onDismissReclassifyResult
  }: Props = $props();

  let open = $state(false);

  let issueCount = $derived(
    (autoRefreshStatus?.failures.length ?? 0) + (reclassifyResult?.failures.length ?? 0)
  );

  function plural(count: number, label: string): string {
    return `${count} ${label}${count === 1 ? "" : "s"}`;
  }

  function autoRefreshLede(status: AutoRefreshStatus): string {
    if (status.is_drain) {
      if (status.phase === "processing_zones") {
        return `Catching up on the backlog: ${plural(status.articles_classified, "article")} classified, ${status.articles_queued} to go.`;
      }
      return `Backlog cleared: ${plural(status.articles_classified, "article")} classified.`;
    }
    if (status.phase === "processing_zones") {
      return `Auto-refresh running: ${status.zones_completed}/${status.zones_total} Xones, ${status.sources_completed}/${status.sources_total} sources.`;
    }
    if (status.phase === "failed") {
      return "Auto-refresh stopped before it could finish.";
    }
    return `Auto-refresh complete: ${plural(status.articles_fetched, "article")} received, ${status.articles_inserted} new.`;
  }

  function reclassifyLede(result: ClassifyZoneResult): string {
    const parts = [
      `${plural(result.sources_refreshed, "source")} refreshed`,
      `${plural(result.articles_fetched, "article")} fetched`,
      `${plural(result.articles_classified, "article")} classified`
    ];
    if (result.articles_skipped_by_limit > 0) {
      parts.push(`${plural(result.articles_skipped_by_limit, "article")} queued`);
    }
    if (result.failures.length > 0) {
      parts.push(plural(result.failures.length, "issue"));
    }
    return parts.join(", ") + ".";
  }

  function coverage(limit: number, unit: string): string {
    return limit === 0 ? "All unmatched, past 24 hours" : `${limit}${unit}`;
  }
</script>

{#if autoRefreshStatus || reclassifyResult}
  <div class="toolbar-popover-anchor">
    <button
      class="icon-button toolbar-icon-button"
      class:flagged={issueCount > 0}
      type="button"
      onclick={() => (open = !open)}
      aria-expanded={open}
      aria-label="Show last run details"
      title="Last run details"
    >
      <Info size={16} weight="regular" aria-hidden="true" />
    </button>

    {#if open}
      <div class="toolbar-popover" role="dialog" aria-label="Last run details">
        {#if autoRefreshStatus}
          <p class="popover-lede">{autoRefreshLede(autoRefreshStatus)}</p>
          <dl>
            {#if !autoRefreshStatus.is_drain}
              <div>
                <dt>Sources</dt>
                <dd>{autoRefreshStatus.sources_refreshed}/{autoRefreshStatus.sources_total}</dd>
              </div>
            {/if}
            <div>
              <dt>Xones</dt>
              <dd>{autoRefreshStatus.zones_completed}/{autoRefreshStatus.zones_total}</dd>
            </div>
            {#if !autoRefreshStatus.is_drain}
              <div>
                <dt>New articles</dt>
                <dd>{autoRefreshStatus.articles_inserted}</dd>
              </div>
            {/if}
            <div>
              <dt>Classified</dt>
              <dd>{autoRefreshStatus.articles_classified}</dd>
            </div>
            <div>
              <dt>Visible matches</dt>
              <dd>{autoRefreshStatus.articles_relevant}</dd>
            </div>
            <div>
              <dt>Queued</dt>
              <dd>{autoRefreshStatus.articles_queued}</dd>
            </div>
            <div>
              <dt>Coverage</dt>
              <dd>{coverage(autoRefreshStatus.processing_limit, " per Xone")}</dd>
            </div>
          </dl>
          {#if autoRefreshStatus.failures.length > 0}
            <ul>
              {#each autoRefreshStatus.failures as failure}
                <li><strong>{failure.source}</strong> {failure.message}</li>
              {/each}
            </ul>
          {/if}
        {/if}

        {#if reclassifyResult}
          {#if autoRefreshStatus}<hr />{/if}
          <p class="popover-lede">{reclassifyLede(reclassifyResult)}</p>
          <dl>
            <div>
              <dt>Run limit</dt>
              <dd>{coverage(reclassifyResult.processing_limit, "")}</dd>
            </div>
            <div>
              <dt>Visible matches</dt>
              <dd>{reclassifyResult.articles_relevant}</dd>
            </div>
            <div>
              <dt>Skipped by limit</dt>
              <dd>{reclassifyResult.articles_skipped_by_limit}</dd>
            </div>
          </dl>
          {#if reclassifyResult.failures.length > 0}
            <ul>
              {#each reclassifyResult.failures as failure}
                <li><strong>{failure.source}</strong> {failure.message}</li>
              {/each}
            </ul>
          {/if}
        {/if}

        <button
          class="popover-dismiss"
          type="button"
          onclick={() => {
            open = false;
            onDismissAutoRefresh();
            onDismissReclassifyResult();
          }}
        >
          Clear
        </button>
      </div>
    {/if}
  </div>
{/if}
