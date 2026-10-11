<script lang="ts">
  import {
    GearSix,
    DotsSixVertical,
    Moon,
    PencilSimple,
    Plus,
    Sun,
    Trash
  } from "phosphor-svelte";
  import type {
    AiStatus,
    Feed,
    SmartFeedCounts,
    SmartFeedKind,
    Zone
  } from "../lib/types";

  interface Props {
    zones: Zone[];
    feeds: Feed[];
    smartFeedCounts: SmartFeedCounts;
    activeSmartFeed: SmartFeedKind | null;
    selectedZoneId: number | null;
    activeSection: "articles" | "sources";
    loading: boolean;
    onSelectZone: (zone: Zone) => void;
    onSelectSmartFeed: (smartFeed: SmartFeedKind) => void;
    onCreateZone: () => void;
    onEditZone: (zone: Zone) => void;
    onDeleteZone: (zone: Zone) => void;
    onReorderZones: (orderedZoneIds: number[]) => Promise<void>;
    onOpenSources: () => void;
    onOpenSettings: () => void;
    onToggleTheme: () => void;
    theme: "light" | "dark";
    aiStatus: AiStatus | null;
  }

  let {
    zones,
    feeds,
    smartFeedCounts,
    activeSmartFeed,
    selectedZoneId,
    activeSection,
    loading,
    onSelectZone,
    onSelectSmartFeed,
    onCreateZone,
    onEditZone,
    onDeleteZone,
    onReorderZones,
    onOpenSources,
    onOpenSettings,
    onToggleTheme,
    theme,
    aiStatus
  }: Props = $props();

  let draggedZoneId: number | null = $state(null);
  let dropTargetZoneId: number | null = $state(null);
  let dropAfter = $state(false);
  let reorderSaving = $state(false);
  let dragPointerId: number | null = null;
  let pendingDragZoneId: number | null = null;
  let dragStartY = 0;
  let dragStartedOnZone = false;
  let suppressZoneSelectionUntil = 0;

  async function moveZone(zoneId: number, targetZoneId: number, after: boolean) {
    if (reorderSaving || zoneId === targetZoneId) return;
    const orderedIds = zones.map((zone) => zone.id).filter((id) => id !== zoneId);
    const targetIndex = orderedIds.indexOf(targetZoneId);
    if (targetIndex < 0) return;
    orderedIds.splice(targetIndex + (after ? 1 : 0), 0, zoneId);
    reorderSaving = true;
    try {
      await onReorderZones(orderedIds);
    } finally {
      reorderSaving = false;
    }
  }

  function startZonePointerDrag(event: PointerEvent, zoneId: number, fromZone = false) {
    if (reorderSaving || event.button !== 0) return;
    dragPointerId = event.pointerId;
    pendingDragZoneId = zoneId;
    dragStartY = event.clientY;
    dragStartedOnZone = fromZone;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function updateZonePointerDrag(event: PointerEvent) {
    if (event.pointerId !== dragPointerId || pendingDragZoneId === null) return;
    if (draggedZoneId === null) {
      if (Math.abs(event.clientY - dragStartY) < 4) return;
      draggedZoneId = pendingDragZoneId;
      if (dragStartedOnZone) suppressZoneSelectionUntil = performance.now() + 500;
    }
    event.preventDefault();

    const rows = Array.from(document.querySelectorAll<HTMLElement>(".zone-list .nav-row"))
      .filter((row) => Number(row.dataset.zoneId) !== draggedZoneId);
    let closestRow: HTMLElement | null = null;
    let closestDistance = Number.POSITIVE_INFINITY;
    for (const row of rows) {
      const bounds = row.getBoundingClientRect();
      const distance = Math.abs(event.clientY - (bounds.top + bounds.height / 2));
      if (distance < closestDistance) {
        closestDistance = distance;
        closestRow = row;
      }
    }
    if (!closestRow) return;
    const bounds = closestRow.getBoundingClientRect();
    dropTargetZoneId = Number(closestRow.dataset.zoneId);
    dropAfter = event.clientY >= bounds.top + bounds.height / 2;
  }

  function finishZonePointerDrag(event: PointerEvent) {
    if (event.pointerId !== dragPointerId) return;
    const zoneId = draggedZoneId;
    if (zoneId !== null) event.preventDefault();
    const targetZoneId = dropTargetZoneId;
    const after = dropAfter;
    finishZoneDrag();
    if (zoneId !== null && targetZoneId !== null) {
      void moveZone(zoneId, targetZoneId, after);
    }
  }

  function finishZoneDrag() {
    draggedZoneId = null;
    dropTargetZoneId = null;
    dropAfter = false;
    dragPointerId = null;
    pendingDragZoneId = null;
    dragStartY = 0;
    dragStartedOnZone = false;
  }

  function selectZone(event: MouseEvent, zone: Zone) {
    if (performance.now() < suppressZoneSelectionUntil) {
      event.preventDefault();
      return;
    }
    onSelectZone(zone);
  }

  function moveZoneWithKeyboard(event: KeyboardEvent, zoneId: number) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const currentIndex = zones.findIndex((zone) => zone.id === zoneId);
    const targetIndex = currentIndex + (event.key === "ArrowUp" ? -1 : 1);
    if (currentIndex < 0 || targetIndex < 0 || targetIndex >= zones.length) return;
    void moveZone(zoneId, zones[targetIndex].id, event.key === "ArrowDown");
  }

</script>

<aside class="sidebar">
  <div class="brand" data-tauri-drag-region>
    <div>
      <h1>Xone</h1>
      <p class:connected={aiStatus?.connected} class="ai-status">
        {aiStatus?.connected ? "AI connected" : "AI disconnected"}
      </p>
    </div>

    <div class="sidebar-actions" aria-label="Primary actions">
    <button
      class="icon-button primary-icon-button"
      type="button"
      onclick={onOpenSettings}
      aria-label="Open settings"
      title="Settings"
    >
      <GearSix size={19} weight="regular" aria-hidden="true" />
    </button>
    <button
      class="icon-button primary-icon-button"
      type="button"
      onclick={onToggleTheme}
      aria-label={theme === "dark" ? "Use light mode" : "Use dark mode"}
      title={theme === "dark" ? "Use light mode" : "Use dark mode"}
    >
      {#if theme === "dark"}
        <Sun size={17} weight="regular" aria-hidden="true" />
      {:else}
        <Moon size={17} weight="regular" aria-hidden="true" />
      {/if}
    </button>
    </div>
  </div>

  {#if loading}
    <p class="muted">Loading sources...</p>
  {:else}
    <nav aria-label="Smart feeds, Xones, and app sections">
      <section class="nav-group smart-feeds-group">
        <div class="nav-heading smart-feeds-heading">
          <h2>Smart Feeds</h2>
        </div>
        <button
          class="smart-feed-select"
          class:active={activeSmartFeed === "tldr"}
          type="button"
          onclick={() => onSelectSmartFeed("tldr")}
        >
          <span>TL:DR</span>
        </button>
        <button
          class="smart-feed-select"
          class:active={activeSmartFeed === "today"}
          type="button"
          onclick={() => onSelectSmartFeed("today")}
        >
          <span>Today</span>
          <strong>{smartFeedCounts.today}</strong>
        </button>
        <button
          class="smart-feed-select"
          class:active={activeSmartFeed === "unread"}
          type="button"
          onclick={() => onSelectSmartFeed("unread")}
        >
          <span>All Unread</span>
          <strong>{smartFeedCounts.all_unread}</strong>
        </button>
      </section>

      <section class="nav-group xones-group">
        <div class="nav-heading">
          <h2>Xones</h2>
          <button
            class="icon-button"
            type="button"
            onclick={onCreateZone}
            aria-label="Create Xone"
            title="Create Xone"
          >
            <Plus size={16} weight="regular" aria-hidden="true" />
          </button>
        </div>

        {#if zones.length === 0}
          <p class="sidebar-empty">No Xones yet.</p>
        {:else}
          <div class="zone-list" role="list" aria-label="Xones">
            {#each zones as zone (zone.id)}
              <div
                class="nav-row"
                class:dragging={draggedZoneId === zone.id}
                class:drop-before={dropTargetZoneId === zone.id && !dropAfter}
                class:drop-after={dropTargetZoneId === zone.id && dropAfter}
                data-zone-id={zone.id}
                role="listitem"
              >
                <button
                  class="zone-drag-handle"
                  type="button"
                  disabled={reorderSaving}
                  onpointerdown={(event) => startZonePointerDrag(event, zone.id)}
                  onpointermove={updateZonePointerDrag}
                  onpointerup={finishZonePointerDrag}
                  onpointercancel={finishZoneDrag}
                  onkeydown={(event) => moveZoneWithKeyboard(event, zone.id)}
                  aria-label={`Reorder ${zone.name}`}
                  title="Drag to reorder; use arrow keys for precise movement"
                >
                  <DotsSixVertical size={15} weight="bold" aria-hidden="true" />
                </button>
                <button
                  class="nav-select"
                  class:active={zone.id === selectedZoneId}
                  type="button"
                  onpointerdown={(event) => startZonePointerDrag(event, zone.id, true)}
                  onpointermove={updateZonePointerDrag}
                  onpointerup={finishZonePointerDrag}
                  onpointercancel={finishZoneDrag}
                  onclick={(event) => selectZone(event, zone)}
                  title={zone.filter_description}
                >
                  <span>{zone.name}</span>
                  {#if zone.unread_count > 0}
                    <strong>{zone.unread_count}</strong>
                  {/if}
                </button>
                <div class="row-actions">
                  <button
                    class="icon-button"
                    type="button"
                    onclick={(event) => {
                      event.stopPropagation();
                      onEditZone(zone);
                    }}
                    aria-label={`Edit ${zone.name}`}
                    title="Edit Xone"
                  >
                    <PencilSimple size={15} weight="regular" aria-hidden="true" />
                  </button>
                  <button
                    class="icon-button danger-button"
                    type="button"
                    onclick={(event) => {
                      event.stopPropagation();
                      onDeleteZone(zone);
                    }}
                    aria-label={`Delete ${zone.name}`}
                    title="Delete Xone"
                  >
                    <Trash size={15} weight="regular" aria-hidden="true" />
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <section class="nav-group">
        <button
          class="nav-select app-nav-select"
          class:active={activeSection === "sources"}
          type="button"
          onclick={onOpenSources}
        >
          <span>Sources</span>
          <strong>{feeds.length}</strong>
        </button>
      </section>
    </nav>
  {/if}
</aside>
