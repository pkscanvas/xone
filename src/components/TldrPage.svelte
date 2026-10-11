<script lang="ts">
  import { onMount } from "svelte";
  import { ArrowsClockwise, GearSix, Plus } from "phosphor-svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import type { Article, TldrEntry, UserError, Zone } from "../lib/types";

  interface Props {
    zones: Zone[];
    entries: TldrEntry[];
    loading: boolean;
    error: UserError | null;
    onReload: () => Promise<void>;
    onRegenerate: (zoneId?: number | null) => Promise<void>;
    onSelectZone: (zone: Zone) => void;
    onOpenCitation: (article: Article, zoneId: number) => void;
    onCreateZone: () => void;
    onOpenSettings: () => void;
  }

  let { zones, entries, loading, error, onReload, onRegenerate, onSelectZone,
    onOpenCitation, onCreateZone, onOpenSettings }: Props = $props();
  let summaries = $derived(new Map(entries.map((entry) => [entry.zone_id, entry])));
  let generating = $derived(entries.some((entry) => entry.status === "updating"));
  let requested = $state(false);

  onMount(() => {
    // Revalidate the rolling window even when no new feeds arrive.
    const timer = window.setInterval(() => void onReload(), 30_000);
    return () => window.clearInterval(timer);
  });

  async function regenerate(zoneId: number | null = null) {
    requested = true;
    try { await onRegenerate(zoneId); }
    finally { requested = false; }
  }

  function stamp(value: string) {
    return new Date(value).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }

  function citation(entry: TldrEntry, id: number) {
    return entry.citations.find((article) => article.id === id);
  }
</script>

<section class="article-list tldr-page" aria-label="TL:DR briefings">
  <header class="toolbar" data-tauri-drag-region>
    <div class="toolbar-title">
      <h2>TL:DR</h2>
      <p class="toolbar-sub"><span>Past 24 hours</span><span>{zones.length} {zones.length === 1 ? "Xone" : "Xones"}</span></p>
    </div>
    <div class="toolbar-actions">
      <button class="icon-button toolbar-icon-button" type="button"
        aria-label="Regenerate briefings" title="Regenerate briefings from saved articles (⌘R)"
        disabled={requested || loading || zones.length === 0} onclick={() => regenerate()}>
        <ArrowsClockwise size={16} aria-hidden="true" />
      </button>
    </div>
    {#if loading || generating || requested}<div class="tldr-progress" aria-hidden="true"></div>{/if}
  </header>

  <div class="tldr-body" aria-busy={loading}>
    <p class="tldr-disclaimer">AI-generated summaries may contain errors. Check the linked sources for details.</p>
    <ErrorNotice {error} />
    {#if zones.length === 0}
      <div class="tldr-empty">
        <p>Create a Xone to get a daily briefing.</p>
        <button class="icon-button" type="button" aria-label="Create Xone" title="Create Xone" onclick={onCreateZone}>
          <Plus size={18} aria-hidden="true" />
        </button>
      </div>
    {/if}
    {#each zones as zone (zone.id)}
      {@const entry = summaries.get(zone.id)}
      <section class="tldr-zone" aria-labelledby={`tldr-zone-${zone.id}`}>
        <div class="tldr-heading">
          <h3 id={`tldr-zone-${zone.id}`}><button type="button" onclick={() => onSelectZone(zone)} title={`Open ${zone.name}`}>{zone.name}</button></h3>
          {#if entry?.status === "error"}
            <button class="icon-button" type="button" aria-label={`Retry ${zone.name} briefing`} title="Retry briefing"
              disabled={requested} onclick={() => regenerate(zone.id)}><ArrowsClockwise size={15} aria-hidden="true" /></button>
          {:else if entry?.status === "unconfigured"}
            <button class="icon-button" type="button" aria-label="Configure AI in Settings" title="Configure AI in Settings" onclick={onOpenSettings}>
              <GearSix size={16} aria-hidden="true" />
            </button>
          {/if}
        </div>
        {#if entry?.sentences.length}
          <p class="tldr-summary">
            {#each entry.sentences as sentence}
              <span>{sentence.text}</span><span class="tldr-citations">
                {#each sentence.article_ids as id}
                  {@const article = citation(entry, id)}
                  {#if article}
                    <button type="button" aria-label={`Open source: ${article.title}`} title={article.title}
                      onclick={() => onOpenCitation(article, zone.id)}>[{entry.citations.findIndex((item) => item.id === id) + 1}]</button>
                  {/if}
                {/each}
              </span>{" "}
            {/each}
          </p>
        {/if}
        <div class="tldr-meta" aria-live="polite">
          {#if !entry}
            <span>{error ? "Briefing unavailable." : "Loading briefing…"}</span>
          {:else if entry.status === "empty"}
            <span>No matching news in the past 24 hours.</span>
          {:else}
            {#if entry.generated_at}<span>Updated <time datetime={entry.generated_at}>{stamp(entry.generated_at)}</time></span>{/if}
            {#if entry.status === "unconfigured"}<span>Choose an AI provider in Settings to create briefings.</span>
            {:else if entry.status === "error"}<span>{entry.error}</span>
            {:else if entry.status === "updating"}<span>{entry.sentences.length ? "Updating…" : "Preparing briefing…"}</span>{/if}
            {#if entry.stale && entry.sentences.length}<span>New articles pending</span>{/if}
          {/if}
        </div>
      </section>
    {/each}
  </div>
</section>

<style>
  .toolbar { flex-direction: row; align-items: flex-end; }
  .toolbar-actions { width: auto; }
  .tldr-body { padding: 24px var(--column-inset) 48px; }
  .tldr-disclaimer { margin: 0 0 24px; color: var(--ink-3); font-size: 0.75rem; line-height: 1.5; }
  .tldr-zone { margin-bottom: 32px; }
  .tldr-heading { display: flex; align-items: center; gap: 8px; min-height: 24px; }
  h3 { margin: 0; min-width: 0; font-size: 0.9375rem; }
  h3 button { padding: 0; border: 0; background: none; color: var(--ink-1); font: inherit; text-align: left; overflow-wrap: anywhere; cursor: pointer; }
  h3 button:hover { text-decoration: underline; text-underline-offset: 3px; }
  .tldr-summary { margin: 8px 0; font-family: var(--font-editorial); font-size: 1.125rem; line-height: 1.65; color: var(--ink-1); overflow-wrap: anywhere; }
  .tldr-citations { display: inline; white-space: nowrap; margin-left: 3px; }
  .tldr-citations button { padding: 2px; border: 0; background: none; color: var(--ink-3); font-family: var(--font-ui, -apple-system, sans-serif); font-size: 0.6875rem; vertical-align: super; cursor: pointer; }
  .tldr-citations button:hover { color: var(--ink-1); text-decoration: underline; }
  .tldr-meta { display: flex; flex-wrap: wrap; gap: 4px 12px; margin-top: 8px; color: var(--ink-3); font-size: 0.6875rem; line-height: 1.5; }
  .tldr-empty { display: flex; align-items: center; gap: 12px; color: var(--ink-3); }
  .tldr-empty p { margin: 0; font-family: var(--font-editorial); }
  .tldr-progress { position: absolute; bottom: 0; left: 0; width: 100%; height: 1px; background: var(--ink-3); animation: tldr-pulse 1.8s ease-in-out infinite; }
  @keyframes tldr-pulse { 50% { opacity: 0.25; } }
  @media (prefers-reduced-motion: reduce) { .tldr-progress { animation: none; } }
</style>
