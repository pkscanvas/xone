<script lang="ts">
  import type { Article, MatchLevel } from "../lib/types";

  type ReaderContent = {
    body: string | null;
    summary: string | null;
    mode: "body" | "excerpt" | "source";
  };

  interface Props {
    article: Article | null;
    onOpen: (url: string) => void;
  }

  let { article, onOpen }: Props = $props();
  let readerContent = $derived.by(() => buildReaderContent(article));

  function matchLabel(level: MatchLevel): string {
    return level === "high" ? "High AI match" : level === "medium" ? "Medium AI match" : "Low AI match";
  }

  function formatDate(value: string | null): string {
    if (!value) return "No publish date";
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short"
    }).format(new Date(value));
  }

  function buildReaderContent(article: Article | null): ReaderContent {
    if (!article) return { body: null, summary: null, mode: "source" };

    const summary = cleanText(article.summary);
    const content = cleanText(article.content);
    const contentBody = usefulText(content, article.title, summary, 24);

    if (contentBody) {
      return {
        body: contentBody,
        summary: distinctText(summary, article.title, contentBody),
        mode: "body"
      };
    }

    const summaryBody = usefulText(summary, article.title, null, 12);
    return {
      body: summaryBody,
      summary: null,
      mode: summaryBody ? "excerpt" : "source"
    };
  }

  function cleanText(value: string | null | undefined): string | null {
    const text = value?.replace(/\s+/g, " ").trim();
    return text ? text : null;
  }

  function usefulText(
    value: string | null,
    title: string,
    duplicateOf: string | null,
    minWords: number
  ): string | null {
    if (!value) return null;
    if (isMostlySame(value, title) || (duplicateOf && isMostlySame(value, duplicateOf))) {
      return null;
    }

    const wordCount = words(value).length;
    if (wordCount < minWords) return null;
    if (isLinkOnlyPrompt(value, wordCount)) return null;

    return value;
  }

  function distinctText(
    value: string | null,
    title: string,
    body: string | null
  ): string | null {
    if (!value) return null;
    if (isMostlySame(value, title) || (body && isMostlySame(value, body))) return null;
    return value;
  }

  function isMostlySame(left: string, right: string): boolean {
    const leftWords = new Set(words(left));
    const rightWords = new Set(words(right));
    if (leftWords.size === 0 || rightWords.size === 0) return false;

    let overlap = 0;
    for (const word of leftWords) {
      if (rightWords.has(word)) overlap += 1;
    }

    return overlap / Math.min(leftWords.size, rightWords.size) >= 0.85;
  }

  function isLinkOnlyPrompt(value: string, wordCount: number): boolean {
    if (wordCount > 18) return false;
    return /\b(read more|continue reading|full story|view article|click here|open link)\b/i.test(
      value
    );
  }

  function words(value: string): string[] {
    return value.toLowerCase().match(/[a-z0-9]+/g) ?? [];
  }
</script>

<article class="reader">
  {#if article}
    <header>
      <p class="eyebrow">{formatDate(article.published_at ?? article.fetched_at)}</p>
      <h2>{article.title}</h2>
      {#if article.author}
        <p class="byline">By {article.author}</p>
      {/if}
      {#if article.ai_match_level && article.ai_match_level !== "no"}
        <p class="match-level">{matchLabel(article.ai_match_level)}</p>
      {/if}
      <button
        class="icon-button content-icon-button"
        type="button"
        onclick={() => onOpen(article.url)}
        aria-label="Open in browser"
        title="Open in browser"
      >
        <span aria-hidden="true">↗</span>
      </button>
    </header>

    <div class:source-only={readerContent.mode === "source"} class="reader-body">
      <section class="reader-copy" aria-label="Article reader">
        {#if readerContent.body}
          {#if readerContent.summary}
            <p class="summary">{readerContent.summary}</p>
          {/if}
          {#if readerContent.mode === "excerpt"}
            <p class="reader-note">Source excerpt</p>
          {/if}
          <p class="content">{readerContent.body}</p>
        {:else}
          <div class="source-empty">
            <h3>Read this at the source</h3>
            <p>This item does not include a readable article body here.</p>
            <button type="button" onclick={() => onOpen(article.url)}>Open source</button>
          </div>
        {/if}
      </section>

      <aside class="source-panel" aria-label="Source link">
        <p class="eyebrow">Source</p>
        <h3>{article.title}</h3>
        <p>
          {readerContent.body
            ? "Open the original story when you need the full source page."
            : "Open the original story to continue reading."}
        </p>
        <button type="button" onclick={() => onOpen(article.url)}>Open source</button>
      </aside>
    </div>
  {:else}
    <div class="empty-reader">
      <h2>No article selected</h2>
      <p>Select an article to read it or open its source.</p>
    </div>
  {/if}
</article>
