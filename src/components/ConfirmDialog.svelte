<script lang="ts">
  interface Props {
    open: boolean;
    title: string;
    message: string;
    confirmLabel: string;
    loading: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }

  let { open, title, message, confirmLabel, loading, onConfirm, onCancel }: Props = $props();
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => {
      if (event.currentTarget === event.target && !loading) onCancel();
    }}
  >
    <div
      class="confirm-dialog"
      aria-labelledby="confirm-dialog-title"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <header>
        <h2 id="confirm-dialog-title">{title}</h2>
      </header>

      <p>{message}</p>

      <footer>
        <button type="button" onclick={onCancel} disabled={loading}>Cancel</button>
        <button class="danger-submit" type="button" onclick={onConfirm} disabled={loading}>
          {loading ? "Working..." : confirmLabel}
        </button>
      </footer>
    </div>
  </div>
{/if}
