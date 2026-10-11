<script lang="ts">
  import type { UserError } from "../lib/types";

  interface Props {
    error: UserError | null;
    compact?: boolean;
  }

  let { error, compact = false }: Props = $props();
</script>

{#if error}
  <div class:compact class="error" role="alert">
    <p>{error.message}</p>
    {#if error.detail}
      <details>
        <summary>Details</summary>
        <pre>{error.detail}</pre>
      </details>
    {/if}
  </div>
{/if}
