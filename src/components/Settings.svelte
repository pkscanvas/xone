<script lang="ts">
  import { EyeIcon, EyeSlashIcon, LockIcon, LockOpenIcon, X } from "phosphor-svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import type {
    AiModel,
    AiProvider,
    AiProviderConfig,
    AiSettings,
    AppSettings,
    AppearanceSettings,
    ArticleRetentionDays,
    ClassificationProcessingMode,
    UserError
  } from "../lib/types";

  interface Props {
    open: boolean;
    error: UserError | null;
    settings: AiSettings | null;
    appSettings: AppSettings | null;
    appearance: AppearanceSettings;
    onSubmit: (
      settings: AiSettings,
      appSettings: AppSettings,
      appearance: AppearanceSettings
    ) => void;
    onPreviewAppearance: (appearance: AppearanceSettings) => void;
    onFetchModels: (settings: AiSettings) => Promise<AiModel[]>;
    onCountOldArticles: (retentionDays: ArticleRetentionDays) => Promise<number>;
    onDeleteOldArticles: (retentionDays: ArticleRetentionDays) => Promise<number>;
    onClose: () => void;
  }

  const PROVIDERS: { value: AiProvider; label: string }[] = [
    { value: "ollama", label: "Ollama" },
    { value: "lmstudio", label: "LM Studio" },
    { value: "openai", label: "OpenAI" },
    { value: "anthropic", label: "Claude" },
    { value: "groq", label: "Groq" },
    { value: "openrouter", label: "OpenRouter" },
    { value: "custom", label: "Custom server (OpenAI-compatible)" }
  ];
  const SETTINGS_TABS = ["ai", "processing", "appearance"] as const;

  const DEFAULT_CONFIGS: Record<AiProvider, AiProviderConfig> = {
    ollama: providerConfig("llama3.1", "http://localhost:11434"),
    lmstudio: providerConfig("", "http://localhost:1234"),
    openai: providerConfig("gpt-4o-mini", "https://api.openai.com"),
    anthropic: providerConfig("claude-3-5-haiku-latest", "https://api.anthropic.com"),
    groq: providerConfig("llama-3.3-70b-versatile", "https://api.groq.com/openai"),
    openrouter: providerConfig("openai/gpt-4o-mini", "https://openrouter.ai/api/v1"),
    custom: providerConfig("", "")
  };

  let {
    open,
    error,
    settings,
    appSettings,
    appearance,
    onSubmit,
    onPreviewAppearance,
    onFetchModels,
    onCountOldArticles,
    onDeleteOldArticles,
    onClose
  }: Props = $props();

  let provider = $state<AiProvider>("ollama");
  let activeTab = $state<"ai" | "processing" | "appearance">("ai");
  let providerConfigs = $state<Record<AiProvider, AiProviderConfig>>(cloneConfigs(DEFAULT_CONFIGS));
  let savedApiKeys = $state<Record<AiProvider, string>>(providerApiKeys(DEFAULT_CONFIGS));
  let previousProvider = $state<AiProvider>("ollama");
  let apiKey = $state("");
  let model = $state(DEFAULT_CONFIGS.ollama.model);
  let baseUrl = $state(DEFAULT_CONFIGS.ollama.base_url);
  let maxTokens = $state("");
  let temperature = $state("");
  let topP = $state("");
  let showApiKey = $state(false);
  let apiKeyLocked = $state(false);
  let modelsByProvider = $state<Partial<Record<AiProvider, AiModel[]>>>({});
  let modelsLoadingProvider = $state<AiProvider | null>(null);
  let modelsError = $state("");
  let refreshIntervalMinutes = $state(30);
  let classificationProcessingMode = $state<ClassificationProcessingMode>("all_unclassified");
  let classificationProcessingLimit = $state(25);
  let articleRetentionDays = $state<ArticleRetentionDays>(30);
  let readingFont = $state<AppearanceSettings["reading_font"]>("editorial");
  let textSize = $state<AppearanceSettings["text_size"]>("standard");
  let cleanupCount = $state<number | null>(null);
  let cleanupLoading = $state(false);
  let cleanupError = $state("");
  let cleanupResult = $state("");
  let autoFetchTimer: number | null = null;

  $effect(() => {
    if (!open || !settings) return;
    const nextConfigs = cloneConfigs(settings.providers);
    const nextSavedApiKeys = providerApiKeys(nextConfigs);
    provider = settings.provider;
    providerConfigs = nextConfigs;
    savedApiKeys = nextSavedApiKeys;
    loadProviderConfig(settings.provider, nextConfigs[settings.provider]);
    previousProvider = settings.provider;
    modelsByProvider = {};
    modelsError = "";
    activeTab = "ai";
    scheduleAutoModelFetch();
  });

  $effect(() => {
    if (!open) return;
    readingFont = appearance.reading_font;
    textSize = appearance.text_size;
  });

  $effect(() => {
    if (!open) return;
    onPreviewAppearance({ reading_font: readingFont, text_size: textSize });
  });

  $effect(() => {
    if (!open || !appSettings) return;
    refreshIntervalMinutes = appSettings.refresh_interval_minutes;
    classificationProcessingMode = appSettings.classification_processing_mode;
    classificationProcessingLimit = appSettings.classification_processing_limit;
    articleRetentionDays = appSettings.article_retention_days;
    resetCleanupState();
  });

  $effect(() => {
    if (!open || provider === previousProvider) return;
    const nextConfigs = {
      ...providerConfigs,
      [previousProvider]: currentProviderConfig()
    };
    providerConfigs = nextConfigs;
    loadProviderConfig(provider, nextConfigs[provider] ?? DEFAULT_CONFIGS[provider]);
    previousProvider = provider;
    modelsError = "";
    scheduleAutoModelFetch();
  });

  function providerConfig(model: string, baseUrl: string): AiProviderConfig {
    return {
      api_key: "",
      model,
      base_url: baseUrl,
      max_tokens: null,
      temperature: null,
      top_p: null
    };
  }

  function cloneConfigs(
    configs: Record<AiProvider, AiProviderConfig>
  ): Record<AiProvider, AiProviderConfig> {
    return Object.fromEntries(
      PROVIDERS.map(({ value }) => [value, { ...(configs[value] ?? DEFAULT_CONFIGS[value]) }])
    ) as Record<AiProvider, AiProviderConfig>;
  }

  function providerApiKeys(
    configs: Record<AiProvider, AiProviderConfig>
  ): Record<AiProvider, string> {
    return Object.fromEntries(
      PROVIDERS.map(({ value }) => [value, configs[value]?.api_key ?? ""])
    ) as Record<AiProvider, string>;
  }

  function loadProviderConfig(nextProvider: AiProvider, config: AiProviderConfig) {
    const nextConfig = config ?? DEFAULT_CONFIGS[nextProvider];
    const nextApiKey = nextConfig.api_key;
    apiKey = nextApiKey;
    model = nextConfig.model;
    baseUrl = nextConfig.base_url;
    maxTokens = nextConfig.max_tokens?.toString() ?? "";
    temperature = nextConfig.temperature?.toString() ?? "";
    topP = nextConfig.top_p?.toString() ?? "";
    showApiKey = false;
    apiKeyLocked = nextApiKey.trim().length > 0;
  }

  function optionalNumber(value: string): number | null {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const parsed = Number(trimmed);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function currentProviderConfig(): AiProviderConfig {
    return {
      api_key: apiKey,
      model,
      base_url: baseUrl,
      max_tokens: optionalNumber(maxTokens),
      temperature: optionalNumber(temperature),
      top_p: optionalNumber(topP)
    };
  }

  function currentSettings(): AiSettings {
    return {
      provider,
      providers: {
        ...providerConfigs,
        [provider]: currentProviderConfig()
      }
    };
  }

  function providerNeedsApiKey(candidate: AiProvider = provider): boolean {
    return ["openai", "anthropic", "groq", "openrouter"].includes(candidate);
  }

  function providerHasOptionalApiKey(): boolean {
    return provider === "lmstudio" || provider === "custom";
  }

  function providerUsesEndpoint(): boolean {
    return provider === "ollama" || provider === "lmstudio" || provider === "custom";
  }

  function providerUsesTopModelSelect(): boolean {
    return provider !== "custom";
  }

  function settingsIncomplete(): boolean {
    return (
      !model.trim() ||
      !baseUrl.trim() ||
      (providerNeedsApiKey() && !apiKey.trim())
    );
  }

  function endpointHelp(): string {
    if (provider === "ollama") return "Leave the default unless Ollama is running on another host.";
    if (provider === "lmstudio") return "LM Studio server root. Xone uses its OpenAI-compatible API.";
    return "Include /v1 when your OpenAI-compatible server requires it.";
  }

  function visibleModels(): AiModel[] {
    return modelsByProvider[provider] ?? [];
  }

  function providerCanFetchModels(): boolean {
    if (provider === "custom" || !baseUrl.trim()) return false;
    if (!providerNeedsApiKey()) return true;
    const savedKey = savedApiKeys[provider]?.trim() ?? "";
    return savedKey.length > 0 && apiKey.trim() === savedKey;
  }

  function errorMessage(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  function scheduleAutoModelFetch(force = false) {
    if (autoFetchTimer !== null) window.clearTimeout(autoFetchTimer);
    autoFetchTimer = window.setTimeout(() => {
      autoFetchTimer = null;
      if (!open || provider === "custom" || (!force && visibleModels().length > 0)) return;
      if (!providerCanFetchModels()) return;
      void fetchModels();
    }, 0);
  }

  function refreshModelsAfterConfigurationChange() {
    modelsByProvider = { ...modelsByProvider, [provider]: [] };
    scheduleAutoModelFetch(true);
  }

  async function fetchModels() {
    if (modelsLoadingProvider !== null || !providerCanFetchModels()) return;
    const fetchSettings = currentSettings();
    const fetchingProvider = fetchSettings.provider;
    modelsLoadingProvider = fetchingProvider;
    modelsError = "";
    try {
      const fetched = await onFetchModels(fetchSettings);
      modelsByProvider = { ...modelsByProvider, [fetchingProvider]: fetched };
      if (provider === fetchingProvider && !model.trim() && fetched.length > 0) {
        model = fetched[0].id;
      }
    } catch (err) {
      if (provider === fetchingProvider) modelsError = errorMessage(err);
    } finally {
      modelsLoadingProvider = null;
      if (provider !== fetchingProvider) scheduleAutoModelFetch();
    }
  }

  function resetCleanupState() {
    cleanupCount = null;
    cleanupError = "";
    cleanupResult = "";
  }

  async function previewCleanup() {
    if (articleRetentionDays === 0 || cleanupLoading) return;
    cleanupLoading = true;
    cleanupError = "";
    cleanupResult = "";
    try {
      cleanupCount = await onCountOldArticles(articleRetentionDays);
    } catch (err) {
      cleanupError = errorMessage(err);
    } finally {
      cleanupLoading = false;
    }
  }

  async function confirmCleanup() {
    if (!cleanupCount || cleanupLoading) return;
    cleanupLoading = true;
    cleanupError = "";
    try {
      const deleted = await onDeleteOldArticles(articleRetentionDays);
      cleanupCount = null;
      cleanupResult = `Deleted ${deleted.toLocaleString()} old ${deleted === 1 ? "article" : "articles"}.`;
    } catch (err) {
      cleanupError = errorMessage(err);
    } finally {
      cleanupLoading = false;
    }
  }

  function submit() {
    if (settingsIncomplete()) return;
    onSubmit(
      currentSettings(),
      {
        refresh_interval_minutes: Math.max(5, Math.round(refreshIntervalMinutes || 30)),
        classification_processing_mode: classificationProcessingMode,
        classification_processing_limit: Math.min(
          100,
          Math.max(1, Math.round(classificationProcessingLimit || 25))
        ),
        article_retention_days: articleRetentionDays
      },
      currentAppearance()
    );
  }

  function currentAppearance(): AppearanceSettings {
    return { reading_font: readingFont, text_size: textSize };
  }

  function handleTabKeydown(event: KeyboardEvent) {
    let nextIndex = SETTINGS_TABS.indexOf(activeTab);
    if (event.key === "ArrowRight") nextIndex = (nextIndex + 1) % SETTINGS_TABS.length;
    else if (event.key === "ArrowLeft") {
      nextIndex = (nextIndex - 1 + SETTINGS_TABS.length) % SETTINGS_TABS.length;
    } else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = SETTINGS_TABS.length - 1;
    else return;

    event.preventDefault();
    activeTab = SETTINGS_TABS[nextIndex];
    requestAnimationFrame(() => document.getElementById(`settings-tab-${activeTab}`)?.focus());
  }
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => {
      if (event.currentTarget === event.target) onClose();
    }}
  >
    <form
      class="settings-dialog model-settings-dialog"
      aria-labelledby="settings-title"
      onsubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <header>
        <div>
          <h2 id="settings-title">Settings</h2>
          <p class="dialog-description">Tune Xone's intelligence, processing, and reading experience.</p>
        </div>
        <button
          class="icon-button dialog-close-button"
          type="button"
          onclick={onClose}
          aria-label="Close settings"
          title="Close settings"
        >
          <X size={16} weight="regular" aria-hidden="true" />
        </button>
      </header>

      <div class="settings-tabs" role="tablist" aria-label="Settings sections">
        <button
          id="settings-tab-ai"
          class:active={activeTab === "ai"}
          type="button"
          role="tab"
          aria-selected={activeTab === "ai"}
          aria-controls="settings-panel-ai"
          tabindex={activeTab === "ai" ? 0 : -1}
          onclick={() => (activeTab = "ai")}
          onkeydown={handleTabKeydown}
        >AI</button>
        <button
          id="settings-tab-processing"
          class:active={activeTab === "processing"}
          type="button"
          role="tab"
          aria-selected={activeTab === "processing"}
          aria-controls="settings-panel-processing"
          tabindex={activeTab === "processing" ? 0 : -1}
          onclick={() => (activeTab = "processing")}
          onkeydown={handleTabKeydown}
        >Processing</button>
        <button
          id="settings-tab-appearance"
          class:active={activeTab === "appearance"}
          type="button"
          role="tab"
          aria-selected={activeTab === "appearance"}
          aria-controls="settings-panel-appearance"
          tabindex={activeTab === "appearance" ? 0 : -1}
          onclick={() => (activeTab = "appearance")}
          onkeydown={handleTabKeydown}
        >Appearance</button>
      </div>

      <div class="settings-content">
        {#if activeTab === "ai"}
          <div
          id="settings-panel-ai"
          class="model-settings-section settings-panel"
          role="tabpanel"
          aria-labelledby="settings-tab-ai"
          tabindex="0"
        >
          <h3>AI model</h3>

        <div class="provider-model-block">
          <div class:provider-only={!providerUsesTopModelSelect()} class="provider-model-row">
            <div class="provider-field">
              <label for="ai-provider">AI provider</label>
              <select id="ai-provider" bind:value={provider}>
                {#each PROVIDERS as option}
                  <option value={option.value}>{option.label}</option>
                {/each}
              </select>
            </div>

            {#if providerUsesTopModelSelect()}
              <div class="model-select-field">
                <label for="ai-model">AI model</label>
                <select
                  id="ai-model"
                  bind:value={model}
                  disabled={providerNeedsApiKey() && !providerCanFetchModels()}
                >
                  {#if !model.trim()}
                    <option value="">Select model...</option>
                  {:else if !visibleModels().some((option) => option.id === model)}
                    <option value={model}>{model}</option>
                  {/if}
                  {#each visibleModels() as option}
                    <option value={option.id}>{option.label}</option>
                  {/each}
                </select>
              </div>
              <button
                class="model-refresh-button"
                type="button"
                onclick={fetchModels}
                disabled={modelsLoadingProvider !== null || !providerCanFetchModels()}
                aria-label={`Refresh ${PROVIDERS.find((item) => item.value === provider)?.label} models`}
                title={providerCanFetchModels()
                  ? "Refresh available models"
                  : "Save this provider's API key before loading models"}
              >
                {modelsLoadingProvider === provider ? "Fetching..." : "Refresh models"}
              </button>
            {/if}
          </div>

          {#if modelsError}
            <small class="field-error">{modelsError}</small>
          {:else if providerNeedsApiKey() && !providerCanFetchModels()}
            <small class="field-help">Save this provider's API key before loading models.</small>
          {:else if visibleModels().length > 0}
            <small class="field-help">{visibleModels().length} compatible models available.</small>
          {/if}
        </div>

        {#if providerNeedsApiKey() || providerHasOptionalApiKey()}
          <div class="provider-option">
            <label for="api-key">API key{providerHasOptionalApiKey() ? " (optional)" : ""}</label>
            <div class:has-lock={Boolean(apiKey.trim())} class="secret-field-row">
              <input
                id="api-key"
                type={showApiKey ? "text" : "password"}
                bind:value={apiKey}
                placeholder={providerNeedsApiKey() ? "Enter your API key" : "Leave empty if not required"}
                disabled={apiKeyLocked}
                autocomplete="off"
                onchange={refreshModelsAfterConfigurationChange}
              />
              {#if apiKey.trim()}
                <button
                  class="secret-icon-button"
                  type="button"
                  onclick={() => (apiKeyLocked = !apiKeyLocked)}
                  aria-label={apiKeyLocked ? "Unlock API key" : "Lock API key"}
                  title={apiKeyLocked ? "Unlock API key" : "Lock API key"}
                >
                  {#if apiKeyLocked}
                    <LockIcon size={21} weight="regular" aria-hidden="true" />
                  {:else}
                    <LockOpenIcon size={21} weight="regular" aria-hidden="true" />
                  {/if}
                </button>
              {/if}
              <button
                class="secret-icon-button"
                type="button"
                onclick={() => (showApiKey = !showApiKey)}
                aria-label={showApiKey ? "Hide API key" : "Show API key"}
                title={showApiKey ? "Hide API key" : "Show API key"}
              >
                {#if showApiKey}
                  <EyeSlashIcon size={21} weight="regular" aria-hidden="true" />
                {:else}
                  <EyeIcon size={21} weight="regular" aria-hidden="true" />
                {/if}
              </button>
            </div>
            <small class="field-help provider-help">Saved locally and separately for this provider.</small>
          </div>
        {/if}

        {#if providerUsesEndpoint()}
          <div class="provider-option">
            <label for="base-url">Endpoint URL{provider === "ollama" ? " (optional)" : ""}</label>
            <input
              id="base-url"
              type="url"
              bind:value={baseUrl}
              placeholder={DEFAULT_CONFIGS[provider].base_url || "http://localhost:8000/v1"}
              onchange={refreshModelsAfterConfigurationChange}
            />
            <small class="field-help provider-help">{endpointHelp()}</small>
          </div>
        {/if}

        {#if provider === "custom"}
          <div class="provider-option">
            <label for="custom-model">Model name</label>
            <input id="custom-model" bind:value={model} placeholder="gpt-4, llama-3-70b, etc." />
          </div>

          <details class="advanced-settings">
            <summary>Advanced options</summary>
            <div class="generation-options">
              <label for="max-tokens">Max tokens</label>
              <input id="max-tokens" type="number" min="1" bind:value={maxTokens} placeholder="Provider default" />

              <label for="temperature">Temperature (0–2)</label>
              <input id="temperature" type="number" min="0" max="2" step="0.1" bind:value={temperature} placeholder="Provider default" />

              <label for="top-p">Top P (0–1)</label>
              <input id="top-p" type="number" min="0" max="1" step="0.1" bind:value={topP} placeholder="Provider default" />
            </div>
          </details>
        {/if}

          </div>
        {:else if activeTab === "processing"}
          <div
          id="settings-panel-processing"
          class="settings-panel processing-settings"
          role="tabpanel"
          aria-labelledby="settings-tab-processing"
          tabindex="0"
        >
          <div>
            <h3>News processing</h3>
            <p class="settings-panel-description">
              Control how often Xone checks sources, classifies new stories, and clears its archive.
            </p>
          </div>

          <div class="processing-field">
            <label for="refresh-interval">Auto-refresh interval</label>
            <div class="input-with-suffix">
              <input
                id="refresh-interval"
                type="number"
                min="5"
                step="1"
                bind:value={refreshIntervalMinutes}
              />
              <span>minutes</span>
            </div>
            <small>Assigned sources refresh automatically while Xone is open.</small>
          </div>

          <div class="processing-field">
            <label for="classification-mode">Articles per classification run</label>
            <select id="classification-mode" bind:value={classificationProcessingMode}>
              <option value="all_unclassified">All unclassified from the past 24 hours</option>
              <option value="limited">Use a custom limit</option>
            </select>
            {#if classificationProcessingMode === "limited"}
              <label for="classification-limit">Maximum articles per Xone</label>
              <input
                id="classification-limit"
                type="number"
                min="1"
                max="100"
                step="1"
                bind:value={classificationProcessingLimit}
              />
              <small>Processes the newest unclassified articles first (1–100).</small>
            {:else}
              <small>Classifies every unclassified article from the past 24 hours after sources refresh.</small>
            {/if}
          </div>

          <div class="processing-field retention-field">
            <label for="article-retention">Keep articles for</label>
            <select
              id="article-retention"
              bind:value={articleRetentionDays}
              onchange={resetCleanupState}
            >
              <option value={7}>7 days</option>
              <option value={14}>14 days</option>
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
              <option value={0}>Forever</option>
            </select>
            <small>
              Old articles, including unread ones, are removed once a day. Sources and Xones stay intact.
            </small>

            <div class="retention-cleanup">
              <button
                type="button"
                onclick={previewCleanup}
                disabled={articleRetentionDays === 0 || cleanupLoading}
              >
                {cleanupLoading && cleanupCount === null ? "Checking..." : "Delete old articles now"}
              </button>
              {#if articleRetentionDays === 0}
                <small>Choose a retention period to clean up stored articles.</small>
              {:else}
                <small>Uses this period now. Save to use it for automatic cleanup.</small>
              {/if}
            </div>

            {#if cleanupCount !== null}
              <div class="retention-confirmation" role="status">
                {#if cleanupCount === 0}
                  <p>No articles are old enough to delete.</p>
                  <button type="button" onclick={resetCleanupState}>Done</button>
                {:else}
                  <p>
                    Delete {cleanupCount.toLocaleString()} old {cleanupCount === 1 ? "article" : "articles"}?
                    This cannot be undone.
                  </p>
                  <div>
                    <button type="button" onclick={resetCleanupState} disabled={cleanupLoading}>Cancel</button>
                    <button
                      class="danger-inline-button"
                      type="button"
                      onclick={confirmCleanup}
                      disabled={cleanupLoading}
                    >
                      {cleanupLoading ? "Deleting..." : "Delete articles"}
                    </button>
                  </div>
                {/if}
              </div>
            {/if}
            {#if cleanupError}
              <small class="field-error">{cleanupError}</small>
            {:else if cleanupResult}
              <small class="cleanup-result">{cleanupResult}</small>
            {/if}
          </div>
          </div>
        {:else}
          <div
          id="settings-panel-appearance"
          class="settings-panel appearance-settings"
          role="tabpanel"
          aria-labelledby="settings-tab-appearance"
          tabindex="0"
        >
          <div>
            <h3>Reading appearance</h3>
            <p class="settings-panel-description">
              Choose a restrained type style and a comfortable size for the whole app.
            </p>
          </div>

          <fieldset class="appearance-group">
            <legend>Reading font</legend>
            <div class="font-options">
              <label class:active={readingFont === "editorial"} class="font-option font-editorial-option">
                <input
                  type="radio"
                  name="reading-font"
                  value="editorial"
                  bind:group={readingFont}
                />
                <span>
                  <strong>Editorial</strong>
                  <span class="font-sample">The signal beneath the noise.</span>
                  <small>Newsreader</small>
                </span>
              </label>
              <label class:active={readingFont === "mono"} class="font-option font-mono-option">
                <input
                  type="radio"
                  name="reading-font"
                  value="mono"
                  bind:group={readingFont}
                />
                <span>
                  <strong>Monospace</strong>
                  <span class="font-sample">The signal beneath the noise.</span>
                  <small>IBM Plex Mono</small>
                </span>
              </label>
              <label class:active={readingFont === "digital"} class="font-option font-digital-option">
                <input
                  type="radio"
                  name="reading-font"
                  value="digital"
                  bind:group={readingFont}
                />
                <span>
                  <strong>Digital</strong>
                  <span class="font-sample">The signal beneath the noise.</span>
                  <small>Azeret Mono</small>
                </span>
              </label>
            </div>
          </fieldset>

          <fieldset class="appearance-group">
            <legend>Text size</legend>
            <div class="text-size-options">
              <label class:active={textSize === "small"}>
                <input
                  type="radio"
                  name="text-size"
                  value="small"
                  bind:group={textSize}
                />
                <span>Small</span>
              </label>
              <label class:active={textSize === "standard"}>
                <input
                  type="radio"
                  name="text-size"
                  value="standard"
                  bind:group={textSize}
                />
                <span>Standard</span>
              </label>
              <label class:active={textSize === "large"}>
                <input
                  type="radio"
                  name="text-size"
                  value="large"
                  bind:group={textSize}
                />
                <span>Large</span>
              </label>
            </div>
          </fieldset>

          <div class="appearance-preview" aria-live="polite">
            <span>Preview</span>
            <strong>Good reporting makes complexity feel clear.</strong>
            <p>A concise summary gives the story context without slowing down the read.</p>
          </div>
          </div>
        {/if}

        <ErrorNotice {error} compact />
      </div>

      <footer class="model-settings-footer">
        <button type="button" onclick={onClose}>Cancel</button>
        <button type="submit" disabled={settingsIncomplete()}>Save</button>
      </footer>
    </form>
  </div>
{/if}
