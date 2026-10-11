<script lang="ts">
  import { ArrowClockwiseIcon, SparkleIcon, TrashIcon } from "phosphor-svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import type {
    Feed,
    FilterBriefAnswer,
    FilterBriefAssistResult,
    FilterBriefQuestion,
    FocusLevel,
    UserError,
    Zone
  } from "../lib/types";

  interface Props {
    open: boolean;
    loading: boolean;
    error: UserError | null;
    zone: Zone | null;
    feeds: Feed[];
    assignedFeedIds: number[];
    sourceLimit: number;
    onAssistFilter: (
      filterDescription: string,
      clarifications: FilterBriefAnswer[]
    ) => Promise<FilterBriefAssistResult>;
    onRecommend: (
      filterDescription: string,
      excludedFeedIds: number[],
      limit: number
    ) => Promise<number[]>;
    onRetryBehavior: (zone: Zone) => void;
    onResetBehavior: (zone: Zone) => void;
    onSubmit: (
      name: string,
      filterDescription: string,
      focusLevel: FocusLevel,
      feedIds: number[]
    ) => void;
    onClose: () => void;
  }

  let {
    open,
    loading,
    error,
    zone,
    feeds,
    assignedFeedIds,
    sourceLimit,
    onAssistFilter,
    onRecommend,
    onRetryBehavior,
    onResetBehavior,
    onSubmit,
    onClose
  }: Props = $props();

  let name = $state("");
  let filterDescription = $state("");
  let focusLevel = $state<FocusLevel>("focused");
  let selectedFeedIds = $state<Set<number>>(new Set());
  let sourceQuery = $state("");
  let recommendedFeedIds = $state<number[]>([]);
  let recommendationLoading = $state(false);
  let recommendationError = $state<UserError | null>(null);
  let recommendationMessage = $state<string | null>(null);
  let recommendationRequestId = 0;
  let filterAssistLoading = $state(false);
  let filterAssistError = $state<UserError | null>(null);
  let filterAssistMessage = $state<string | null>(null);
  let filterAssistQuestions = $state<FilterBriefQuestion[]>([]);
  let filterAssistSelections = $state<Record<number, string[]>>({});
  let filterAssistBaseBrief = $state("");
  let filterAssistSuggestion = $state("");
  let filterAssistRequestId = 0;
  let initializedZoneKey: string | null = null;

  const focusLevels: { value: FocusLevel; label: string; description: string }[] = [
    { value: "broad", label: "Broad", description: "Low, medium, and high matches" },
    { value: "focused", label: "Focused", description: "Medium and high matches" },
    { value: "laser", label: "Laser-focused", description: "High matches only" }
  ];

  $effect(() => {
    const nextKey = open ? String(zone?.id ?? "new") : null;
    if (nextKey === initializedZoneKey) return;
    initializedZoneKey = nextKey;
    if (open) {
      name = zone?.name ?? "";
      filterDescription = zone?.filter_description ?? "";
      focusLevel = zone?.focus_level ?? "focused";
      selectedFeedIds = new Set(assignedFeedIds);
      sourceQuery = "";
      recommendedFeedIds = [];
      recommendationLoading = false;
      recommendationError = null;
      recommendationMessage = null;
      recommendationRequestId += 1;
      filterAssistLoading = false;
      filterAssistError = null;
      filterAssistMessage = null;
      filterAssistQuestions = [];
      filterAssistSelections = {};
      filterAssistBaseBrief = "";
      filterAssistSuggestion = "";
      filterAssistRequestId += 1;
    }
  });

  function behaviorProgress(currentZone: Zone): string {
    if (currentZone.behavior_reads_pending >= 10) {
      return `${currentZone.behavior_reads_pending} reads ready for learning.`;
    }
    const prefix = currentZone.behavior_profile
      ? "Next update"
      : "First behavior profile";
    return `${prefix}: ${currentZone.behavior_reads_pending} of 10 new reads.`;
  }

  function behaviorUpdatedAt(value: string | null): string | null {
    if (!value) return null;
    const parsed = new Date(value);
    return Number.isNaN(parsed.getTime()) ? null : parsed.toLocaleString();
  }

  function canResetBehavior(currentZone: Zone): boolean {
    return !!currentZone.behavior_profile ||
      currentZone.behavior_reads_processed > 0 ||
      currentZone.behavior_reads_pending > 0 ||
      !!currentZone.behavior_last_error;
  }

  function toggleFeed(feedId: number) {
    const next = new Set(selectedFeedIds);
    if (next.has(feedId)) {
      next.delete(feedId);
    } else {
      if (next.size >= sourceLimit) return;
      next.add(feedId);
    }
    selectedFeedIds = next;
  }

  type SourceGroup = [string, Feed[]];

  function matchesSource(feed: Feed): boolean {
    const query = sourceQuery.trim().toLocaleLowerCase();
    if (!query) return true;
    return [feed.title, feed.description, feed.site_url, feed.category, feed.region]
      .filter((value): value is string => !!value)
      .some((value) => value.toLocaleLowerCase().includes(query));
  }

  function groupSources(groupBy: (feed: Feed) => string | null): SourceGroup[] {
    const groups = new Map<string, Feed[]>();
    for (const feed of feeds) {
      if (recommendedFeedIds.includes(feed.id) || !matchesSource(feed)) continue;
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

  function recommendedSources(): Feed[] {
    const feedsById = new Map(feeds.map((feed) => [feed.id, feed]));
    return recommendedFeedIds
      .map((feedId) => feedsById.get(feedId))
      .filter((feed): feed is Feed => !!feed);
  }

  function visibleRecommendedSources(): Feed[] {
    return recommendedSources().filter(matchesSource);
  }

  function visibleRecommendedGroups(): SourceGroup[] {
    const groups = new Map<string, Feed[]>();
    for (const feed of visibleRecommendedSources()) {
      const groupName = feed.region?.trim()
        ? `Region · ${feed.region.trim()}`
        : `Topic · ${feed.category?.trim() || "Custom Sources"}`;
      groups.set(groupName, [...(groups.get(groupName) ?? []), feed]);
    }
    return Array.from(groups.entries());
  }

  function selectedCount(sourceFeeds: Feed[]): number {
    return sourceFeeds.filter((feed) => selectedFeedIds.has(feed.id)).length;
  }

  function shouldOpenGroup(sourceFeeds: Feed[]): boolean {
    return !!sourceQuery.trim() || selectedCount(sourceFeeds) > 0;
  }

  function canSelect(feedId: number): boolean {
    return selectedFeedIds.has(feedId) || selectedFeedIds.size < sourceLimit;
  }

  function updateFilter(nextFilter: string) {
    if (nextFilter === filterDescription) return;

    invalidateRecommendations();
    clearFilterAssist();
    filterDescription = nextFilter;
  }

  function invalidateRecommendations() {
    if (recommendedFeedIds.length > 0) {
      const nextSelected = new Set(selectedFeedIds);
      for (const feedId of recommendedFeedIds) nextSelected.delete(feedId);
      selectedFeedIds = nextSelected;
    }
    recommendedFeedIds = [];
    recommendationError = null;
    recommendationMessage = null;
    recommendationLoading = false;
    recommendationRequestId += 1;
  }

  function clearFilterAssist() {
    filterAssistLoading = false;
    filterAssistError = null;
    filterAssistMessage = null;
    filterAssistQuestions = [];
    filterAssistSelections = {};
    filterAssistBaseBrief = "";
    filterAssistSuggestion = "";
    filterAssistRequestId += 1;
  }

  function applyAssistedBrief(brief: string) {
    invalidateRecommendations();
    filterDescription = brief.trim();
    filterAssistQuestions = [];
    filterAssistSelections = {};
    filterAssistBaseBrief = "";
    filterAssistSuggestion = "";
    filterAssistError = null;
    filterAssistMessage = "Signal brief sharpened.";
  }

  async function requestFilterAssist(clarifications: FilterBriefAnswer[] = []) {
    const isFollowUp = clarifications.length > 0;
    const requestedBrief = isFollowUp ? filterAssistBaseBrief : filterDescription.trim();
    if (!requestedBrief || filterAssistLoading || recommendationLoading) return;

    const requestId = ++filterAssistRequestId;
    filterAssistLoading = true;
    filterAssistError = null;
    filterAssistMessage = null;
    if (!isFollowUp) {
      filterAssistQuestions = [];
      filterAssistSelections = {};
      filterAssistBaseBrief = requestedBrief;
      filterAssistSuggestion = "";
    }

    try {
      const result = await onAssistFilter(requestedBrief, clarifications);
      if (requestId !== filterAssistRequestId) return;

      if (!isFollowUp && result.questions.length > 0) {
        filterAssistQuestions = result.questions;
        filterAssistSuggestion = result.brief;
        filterAssistSelections = {};
      } else {
        applyAssistedBrief(result.brief);
      }
    } catch (err) {
      if (requestId !== filterAssistRequestId) return;
      filterAssistError = {
        message: "Could not sharpen the signal brief.",
        detail: err instanceof Error ? err.message : String(err)
      };
    } finally {
      if (requestId === filterAssistRequestId) filterAssistLoading = false;
    }
  }

  function toggleClarification(questionIndex: number, option: string) {
    const selected = filterAssistSelections[questionIndex] ?? [];
    filterAssistSelections = {
      ...filterAssistSelections,
      [questionIndex]: selected.includes(option)
        ? selected.filter((item) => item !== option)
        : [...selected, option]
    };
  }

  function selectedClarifications(): FilterBriefAnswer[] {
    return filterAssistQuestions.flatMap((question, index) => {
      const selectedOptions = filterAssistSelections[index] ?? [];
      return selectedOptions.length > 0
        ? [{ question: question.question, selected_options: selectedOptions }]
        : [];
    });
  }

  function skipClarifications() {
    if (!filterAssistSuggestion) return;
    applyAssistedBrief(filterAssistSuggestion);
  }

  async function requestRecommendations() {
    const requestedFilter = filterDescription.trim();
    const previousRecommendations = new Set(recommendedFeedIds);
    const manualFeedIds = Array.from(selectedFeedIds).filter(
      (feedId) => !previousRecommendations.has(feedId)
    );
    const remaining = sourceLimit - manualFeedIds.length;
    if (!requestedFilter || remaining <= 0 || recommendationLoading) return;

    const requestId = ++recommendationRequestId;
    recommendationLoading = true;
    recommendationError = null;
    recommendationMessage = null;

    try {
      const recommendedIds = await onRecommend(requestedFilter, manualFeedIds, remaining);
      if (requestId !== recommendationRequestId || requestedFilter !== filterDescription.trim()) {
        return;
      }

      const availableFeedIds = new Set(feeds.map((feed) => feed.id));
      const manualIds = new Set(manualFeedIds);
      const nextRecommendations: number[] = [];
      for (const feedId of recommendedIds) {
        if (
          nextRecommendations.length >= remaining ||
          !availableFeedIds.has(feedId) ||
          manualIds.has(feedId) ||
          nextRecommendations.includes(feedId)
        ) {
          continue;
        }
        nextRecommendations.push(feedId);
      }

      recommendedFeedIds = nextRecommendations;
      selectedFeedIds = new Set([...manualFeedIds, ...nextRecommendations]);
      recommendationMessage =
        nextRecommendations.length === 0 ? "No strongly aligned sources found." : null;
    } catch (err) {
      if (requestId !== recommendationRequestId) return;
      recommendationError = {
        message: "Could not recommend sources.",
        detail: err instanceof Error ? err.message : String(err)
      };
    } finally {
      if (requestId === recommendationRequestId) recommendationLoading = false;
    }
  }

  function close() {
    recommendationRequestId += 1;
    recommendationLoading = false;
    filterAssistRequestId += 1;
    filterAssistLoading = false;
    onClose();
  }

  function submit() {
    const trimmedName = name.trim();
    const trimmedFilter = filterDescription.trim();
    if (
      !trimmedName ||
      !trimmedFilter ||
      loading ||
      recommendationLoading ||
      filterAssistLoading ||
      filterAssistQuestions.length > 0 ||
      selectedFeedIds.size > sourceLimit
    )
      return;
    onSubmit(trimmedName, trimmedFilter, focusLevel, Array.from(selectedFeedIds));
  }
</script>

{#snippet sourceCheckboxes(sourceFeeds: Feed[])}
  {#each sourceFeeds as feed}
    <label>
      <input
        type="checkbox"
        checked={selectedFeedIds.has(feed.id)}
        disabled={loading || !canSelect(feed.id)}
        onchange={() => toggleFeed(feed.id)}
      />
      <span>
        <strong>{feed.title}</strong>
      </span>
    </label>
  {/each}
{/snippet}

{#snippet sourceGroups(groups: SourceGroup[])}
  {#each groups as [groupName, groupFeeds]}
    <details class="source-checkbox-group" open={shouldOpenGroup(groupFeeds)}>
      <summary>
        <strong>{groupName}</strong>
        <small>
          {#if selectedCount(groupFeeds) > 0}{selectedCount(groupFeeds)} selected · {/if}{groupFeeds.length}
          {groupFeeds.length === 1 ? "source" : "sources"}
        </small>
      </summary>
      <div class="source-checkbox-items">
        {@render sourceCheckboxes(groupFeeds)}
      </div>
    </details>
  {/each}
{/snippet}

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => {
      if (event.currentTarget === event.target && !loading) close();
    }}
  >
    <form
      class="zone-dialog"
      aria-labelledby="zone-editor-title"
      onsubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <header>
        <h2 id="zone-editor-title">{zone ? "Edit Xone" : "Create Xone"}</h2>
        <button type="button" onclick={close} disabled={loading} aria-label="Close Xone editor">
          Close
        </button>
      </header>

      <label for="zone-name">Name</label>
      <input id="zone-name" bind:value={name} placeholder="Tech" disabled={loading} required />

      <div class="filter-brief-heading">
        <label for="zone-filter">Signal brief</label>
        <button
          type="button"
          class="filter-assist-button"
          onclick={() => requestFilterAssist()}
          aria-label={filterAssistLoading ? "Sharpening signal brief" : "Sharpen signal brief with AI"}
          title={filterAssistLoading ? "Sharpening signal brief" : "Sharpen signal brief with AI"}
          disabled={loading ||
            filterAssistLoading ||
            recommendationLoading ||
            filterAssistQuestions.length > 0 ||
            !filterDescription.trim()}
        >
          <SparkleIcon size={13} weight="fill" aria-hidden="true" />
          {filterAssistLoading ? "Sharpening..." : "Sharpen with AI"}
        </button>
      </div>
      <div class="filter-brief-field">
        <textarea
          id="zone-filter"
          value={filterDescription}
          oninput={(event) => updateFilter(event.currentTarget.value)}
          placeholder="e.g. Major Mac app launches and meaningful updates; skip rumors and minor patches"
          rows="3"
          maxlength="4000"
          disabled={loading || filterAssistLoading}
          required
        ></textarea>
      </div>
      {#if filterAssistMessage}
        <p class="filter-assist-message">{filterAssistMessage}</p>
      {/if}
      <ErrorNotice error={filterAssistError} compact />

      {#if filterAssistQuestions.length > 0}
        <section class="filter-clarifications" aria-labelledby="filter-clarifications-title">
          <header>
            <div>
              <strong id="filter-clarifications-title">Quick tune-up</strong>
              <small>Select all that fit. Skip uses AI's best judgment.</small>
            </div>
          </header>
          {#each filterAssistQuestions as question, questionIndex}
            <fieldset class="filter-clarification-question">
              <legend>{question.question}</legend>
              <div class="filter-clarification-options">
                {#each question.options as option}
                  <label
                    class:active={(filterAssistSelections[questionIndex] ?? []).includes(option)}
                  >
                    <input
                      type="checkbox"
                      checked={(filterAssistSelections[questionIndex] ?? []).includes(option)}
                      onchange={() => toggleClarification(questionIndex, option)}
                      disabled={loading || filterAssistLoading}
                    />
                    <span>{option}</span>
                  </label>
                {/each}
              </div>
            </fieldset>
          {/each}
          <footer>
            <button
              type="button"
              onclick={skipClarifications}
              disabled={loading || filterAssistLoading}
            >
              Skip · use best
            </button>
            <button
              type="button"
              class="apply-clarifications-button"
              onclick={() => requestFilterAssist(selectedClarifications())}
              disabled={loading || filterAssistLoading || selectedClarifications().length === 0}
            >
              {filterAssistLoading ? "Finishing..." : "Use my choices"}
            </button>
          </footer>
        </section>
      {/if}

      {#if zone}
        <section class="behavior-learning" aria-labelledby="behavior-learning-title">
          <header>
            <div>
              <strong id="behavior-learning-title">Learned behavior</strong>
              <small>Secondary guidance inferred from articles you open.</small>
            </div>
            <div class="behavior-learning-actions">
              {#if zone.behavior_reads_pending >= 10 || zone.behavior_last_error}
                <button
                  type="button"
                  class="behavior-icon-button"
                  onclick={() => onRetryBehavior(zone)}
                  disabled={loading}
                  aria-label="Retry learned behavior update"
                  title="Retry learned behavior update"
                >
                  <ArrowClockwiseIcon size={16} aria-hidden="true" />
                </button>
              {/if}
              <button
                type="button"
                class="behavior-icon-button danger"
                onclick={() => onResetBehavior(zone)}
                disabled={loading || !canResetBehavior(zone)}
                aria-label="Reset learned behavior"
                title="Reset learned behavior"
              >
                <TrashIcon size={16} aria-hidden="true" />
              </button>
            </div>
          </header>

          {#if zone.behavior_profile}
            <p class="behavior-profile">{zone.behavior_profile}</p>
          {:else}
            <p class="behavior-empty">No learned behavior yet.</p>
          {/if}
          <div class="behavior-meta">
            <span>{behaviorProgress(zone)}</span>
            {#if zone.behavior_reads_processed > 0}
              <span>Learned from {zone.behavior_reads_processed} reads.</span>
            {/if}
            {#if behaviorUpdatedAt(zone.behavior_updated_at)}
              <span>Updated {behaviorUpdatedAt(zone.behavior_updated_at)}.</span>
            {/if}
          </div>
          {#if zone.behavior_last_error}
            <ErrorNotice
              error={{
                message: "Behavior update is pending and will retry.",
                detail: zone.behavior_last_error
              }}
              compact
            />
          {/if}
        </section>
      {/if}

      <fieldset class="focus-level-fieldset">
        <legend>Focus level</legend>
        <div class="focus-level-options">
          {#each focusLevels as option}
            <label class:active={focusLevel === option.value}>
              <input
                type="radio"
                name="focus-level"
                value={option.value}
                bind:group={focusLevel}
                disabled={loading}
              />
              <span>
                <strong>{option.label}</strong>
                <small>{option.description}</small>
              </span>
            </label>
          {/each}
        </div>
      </fieldset>

      <fieldset class="assigned-sources-fieldset">
        <legend>Assigned sources</legend>
        <div class="source-assignment-summary">
          <p class:limit-hit={selectedFeedIds.size >= sourceLimit} class="source-limit">
            {selectedFeedIds.size} of {sourceLimit} sources selected
          </p>
          <button
            type="button"
            class="recommend-sources-button"
            onclick={requestRecommendations}
            disabled={loading ||
              recommendationLoading ||
              filterAssistLoading ||
              filterAssistQuestions.length > 0 ||
              !filterDescription.trim() ||
              selectedFeedIds.size >= sourceLimit ||
              feeds.length === 0}
          >
            <SparkleIcon size={13} weight="fill" aria-hidden="true" />
            {recommendationLoading ? "Recommending..." : "Recommend sources"}
          </button>
        </div>
        {#if recommendationMessage}
          <p class="recommendation-message">{recommendationMessage}</p>
        {/if}
        <ErrorNotice error={recommendationError} compact />
        {#if feeds.length === 0}
          <p class="empty compact">Add sources before assigning them to a Xone.</p>
        {:else}
          <input
            class="source-search"
            type="search"
            bind:value={sourceQuery}
            placeholder="Search sources"
            aria-label="Search sources"
            disabled={loading}
          />
          <div class="feed-checkboxes">
            {#if visibleRecommendedSources().length > 0}
              <section class="source-checkbox-region recommended-sources">
                <h3>
                  <SparkleIcon size={16} weight="fill" aria-hidden="true" />
                  Recommended
                </h3>
                <div class="recommended-source-groups">
                  {#each visibleRecommendedGroups() as [groupName, groupFeeds]}
                    <section class="recommended-source-group">
                      <div class="recommended-source-group-heading">
                        <strong>{groupName}</strong>
                        <small>{selectedCount(groupFeeds)} selected</small>
                      </div>
                      <div class="source-checkbox-items">
                        {@render sourceCheckboxes(groupFeeds)}
                      </div>
                    </section>
                  {/each}
                </div>
              </section>
            {/if}
            {#if regionalSources().length > 0}
              <section class="source-checkbox-region">
                <h3>Regions</h3>
                {@render sourceGroups(regionalSources())}
              </section>
            {/if}
            {#if topicalSources().length > 0}
              <section class="source-checkbox-region">
                <h3>Topics</h3>
                {@render sourceGroups(topicalSources())}
              </section>
            {/if}
            {#if visibleRecommendedSources().length === 0 && regionalSources().length === 0 && topicalSources().length === 0}
              <p class="empty compact">No sources match “{sourceQuery.trim()}”.</p>
            {/if}
          </div>
        {/if}
      </fieldset>

      <ErrorNotice {error} compact />

      <footer>
        <button type="button" onclick={close} disabled={loading}>Cancel</button>
        <button
          type="submit"
          disabled={loading ||
            recommendationLoading ||
            filterAssistLoading ||
            filterAssistQuestions.length > 0 ||
            !name.trim() ||
            !filterDescription.trim() ||
            selectedFeedIds.size > sourceLimit}
        >
          {loading ? "Saving..." : "Save Xone"}
        </button>
      </footer>
    </form>
  </div>
{/if}
