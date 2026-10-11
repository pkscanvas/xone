<script lang="ts">
  import { untrack } from "svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import type { Feed, UserError } from "../lib/types";

  const NEW_CATEGORY = "__new_category__";
  const NEW_REGION = "__new_region__";

  interface Props {
    open: boolean;
    loading: boolean;
    error: UserError | null;
    feeds: Feed[];
    onSubmit: (
      url: string,
      displayName: string | null,
      category: string,
      region: string | null
    ) => void;
    onClose: () => void;
  }

  let { open, loading, error, feeds, onSubmit, onClose }: Props = $props();
  let url = $state("");
  let publisherName = $state("");
  let categoryChoice = $state("");
  let newCategory = $state("");
  let regionChoice = $state("");
  let newRegion = $state("");

  function existingCategories(): string[] {
    return Array.from(
      new Set(feeds.map((feed) => feed.category?.trim() || "Custom Sources"))
    ).sort((left, right) => left.localeCompare(right));
  }

  function selectedCategory(): string {
    return categoryChoice === NEW_CATEGORY ? newCategory.trim() : categoryChoice.trim();
  }

  function existingRegions(): string[] {
    return Array.from(
      new Set(feeds.map((feed) => feed.region?.trim()).filter((region): region is string => !!region))
    ).sort((left, right) => left.localeCompare(right));
  }

  function selectedRegion(): string {
    return regionChoice === NEW_REGION ? newRegion.trim() : regionChoice.trim();
  }

  function isGoogleNewsSearch(): boolean {
    try {
      const parsed = new URL(url.trim());
      return parsed.hostname === "news.google.com" && parsed.pathname === "/rss/search";
    } catch {
      return false;
    }
  }

  $effect(() => {
    if (open) {
      url = "";
      publisherName = "";
      newCategory = "";
      newRegion = "";
      categoryChoice = untrack(() => existingCategories()[0] ?? NEW_CATEGORY);
      regionChoice = untrack(() => existingRegions()[0] ?? NEW_REGION);
    }
  });

  function submit() {
    const trimmedUrl = url.trim();
    const displayName = isGoogleNewsSearch() ? publisherName.trim() : null;
    const category = selectedCategory();
    const region = category === "General News" ? selectedRegion() : null;
    if (
      !trimmedUrl ||
      (isGoogleNewsSearch() && !displayName) ||
      !category ||
      (category === "General News" && !region) ||
      loading
    )
      return;
    onSubmit(trimmedUrl, displayName, category, region);
  }
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => {
      if (event.currentTarget === event.target && !loading) onClose();
    }}
  >
    <form
      class="add-feed-dialog"
      aria-labelledby="add-feed-title"
      onsubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <header>
        <h2 id="add-feed-title">Add Source</h2>
        <button type="button" onclick={onClose} disabled={loading} aria-label="Close add source">
          Close
        </button>
      </header>

      <label for="feed-url">RSS URL</label>
      <input
        id="feed-url"
        type="url"
        bind:value={url}
        placeholder="https://example.com/feed.xml"
        disabled={loading}
        required
      />

      {#if isGoogleNewsSearch()}
        <label for="feed-publisher-name">Publisher name</label>
        <input
          id="feed-publisher-name"
          type="text"
          bind:value={publisherName}
          placeholder="e.g. Nikkei Asia"
          disabled={loading}
          required
        />
        <p class="feed-field-hint">Shown instead of the Google News search query.</p>
      {/if}

      <label for="feed-category">Category</label>
      <select id="feed-category" bind:value={categoryChoice} disabled={loading} required>
        {#each existingCategories() as category}
          <option value={category}>{category}</option>
        {/each}
        <option value={NEW_CATEGORY}>Create new category...</option>
      </select>

      {#if categoryChoice === NEW_CATEGORY}
        <label for="new-feed-category">New category name</label>
        <input
          id="new-feed-category"
          type="text"
          bind:value={newCategory}
          placeholder="e.g. Local News"
          disabled={loading}
          required
        />
      {/if}

      {#if selectedCategory() === "General News"}
        <label for="feed-region">Region</label>
        <select id="feed-region" bind:value={regionChoice} disabled={loading} required>
          {#each existingRegions() as region}
            <option value={region}>{region}</option>
          {/each}
          <option value={NEW_REGION}>Create new region...</option>
        </select>

        {#if regionChoice === NEW_REGION}
          <label for="new-feed-region">New region name</label>
          <input
            id="new-feed-region"
            type="text"
            bind:value={newRegion}
            placeholder="e.g. South Asia"
            disabled={loading}
            required
          />
        {/if}
      {/if}

      <ErrorNotice {error} compact />

      <footer>
        <button type="button" onclick={onClose} disabled={loading}>Cancel</button>
        <button
          type="submit"
          disabled={loading ||
            !url.trim() ||
            (isGoogleNewsSearch() && !publisherName.trim()) ||
            !selectedCategory() ||
            (selectedCategory() === "General News" && !selectedRegion())}
        >
          {loading ? "Adding..." : "Add Source"}
        </button>
      </footer>
    </form>
  </div>
{/if}
