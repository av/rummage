<script lang="ts">
  import { KeyRound } from '@lucide/svelte';
  import { tick } from 'svelte';
  import { setToken, token } from '../lib/api';
  import { app } from '../lib/app.svelte';

  let value = $state('');
  let input = $state<HTMLInputElement>();
  const retry = !!token();

  $effect(() => {
    tick().then(() => input?.focus());
  });

  async function save(e: SubmitEvent) {
    e.preventDefault();
    setToken(value.trim());
    app.needToken = false;
    await app.loadIndex();
    if (!app.needToken && app.active) app.runSearch();
  }
</script>

<div class="scrim" role="presentation"></div>
<div class="modal" role="dialog" aria-modal="true" aria-labelledby="token-title">
  <KeyRound size={24} />
  <h2 id="token-title">{retry ? 'That token did not work' : 'This rummage needs an access token'}</h2>
  <p>Paste the server's <code>server.auth_token</code>. It is kept in this browser only.</p>
  <form onsubmit={save}>
    <label class="sr-only" for="token">Access token</label>
    <input id="token" class="input" type="password" autocomplete="current-password" bind:this={input} bind:value />
    <button class="btn primary" type="submit" disabled={!value.trim()}>Continue</button>
  </form>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 70;
    background: var(--scrim);
  }
  .modal {
    position: fixed;
    z-index: 71;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 32px));
    padding: 24px;
    border-radius: 16px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-lg);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    margin: 4px 0 0;
    font-size: 17px;
  }
  p {
    margin: 0 0 6px;
    color: var(--muted);
    font-size: 14px;
  }
  code {
    font: 12.5px var(--mono);
  }
  form {
    display: flex;
    gap: 8px;
  }
  form .input {
    flex: 1;
  }
</style>
