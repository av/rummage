<script lang="ts">
  import { X, Pause, Play, RefreshCw, RotateCcw, Trash2, FolderTree, SlidersHorizontal, TriangleAlert } from '@lucide/svelte';
  import { tick } from 'svelte';
  import { api, ApiError, type Folder, type FolderProgress } from '../lib/api';
  import { app } from '../lib/app.svelte';
  import { dirname, eta, num, plural } from '../lib/format';
  import DirBrowser from './DirBrowser.svelte';

  let { onclose }: { onclose: () => void } = $props();

  let path = $state('');
  let addError = $state('');
  let adding = $state(false);
  let browsing = $state(false);
  let editing = $state<string | null>(null);
  let excludes = $state('');
  let confirming = $state<string | null>(null);
  let rowError = $state<Record<string, string>>({});
  let filesUrl = $state(app.filesUrl);
  let savedFilesUrl = $state(false);
  let pathInput = $state<HTMLInputElement>();
  let problemsTitle = $state<HTMLElement>();

  const first = $derived(app.foldersLoaded && !app.folders.length);
  const fromEnv = $derived(app.folders.some((f) => f.from_env));
  const s = $derived(app.status);
  const paused = $derived(s?.state === 'paused');
  const pending = $derived(s ? s.queued || s.totals.pending : 0);
  const problems = $derived(app.problems?.total ? app.problems : null);

  // Opened to show the files that could not be read: focus goes there instead of the path box.
  const toProblems = app.drawerFocus === 'problems';

  $effect(() => {
    if (first) browsing = true;
    tick().then(() => {
      if (!toProblems) pathInput?.focus();
    });
  });

  // Go to the list of those files once it is there.
  $effect(() => {
    if (app.drawerFocus === 'problems' && problemsTitle) {
      problemsTitle.scrollIntoView({ block: 'start' });
      problemsTitle.focus({ preventScroll: true });
      app.drawerFocus = '';
    }
  });

  /** A folder's live progress from the index status, or its counts from the folder list. */
  function progress(f: Folder): FolderProgress {
    return app.status?.folders.find((p) => p.id === f.id) ?? { id: f.id, path: f.path, counts: f.counts, scanning: false, eta_secs: null };
  }

  function showProblems() {
    app.drawerFocus = 'problems';
    app.refreshProblems();
  }

  async function add(p: string) {
    addError = '';
    adding = true;
    try {
      await api.addFolder(p.trim());
      path = '';
      await app.refreshFolders();
      app.say('Folder added. Indexing starts now.');
    } catch (e) {
      addError = e instanceof ApiError ? e.message : String(e);
    } finally {
      adding = false;
    }
  }

  async function act(f: Folder, what: () => Promise<unknown>, done?: string) {
    rowError = { ...rowError, [f.id]: '' };
    try {
      await what();
      await app.refreshFolders();
      if (done) app.say(done);
    } catch (e) {
      rowError = { ...rowError, [f.id]: e instanceof ApiError ? e.message : String(e) };
    }
  }

  function edit(f: Folder) {
    editing = editing === f.id ? null : f.id;
    excludes = f.exclude.join('\n');
  }

  async function saveExcludes(f: Folder) {
    const list = excludes
      .split('\n')
      .map((s) => s.trim())
      .filter(Boolean);
    await act(f, () => api.updateFolder(f.id, { exclude: list }), 'Exclusions saved');
    if (!rowError[f.id]) editing = null;
  }

  async function saveFilesUrl() {
    try {
      const s = await api.patchSettings({ ui: { files_url: filesUrl.trim() } });
      app.filesUrl = s.settings.ui?.files_url ?? '';
      savedFilesUrl = true;
      setTimeout(() => (savedFilesUrl = false), 1600);
    } catch (e) {
      app.say(e instanceof ApiError ? e.message : String(e));
    }
  }

  const base = (p: string) => p.split('/').filter(Boolean).pop() ?? p;
  const parent = (p: string) => p.slice(0, p.length - base(p).length).replace(/\/$/, '') || '/';
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="drawer" role="dialog" aria-modal="true" aria-labelledby="drawer-title">
  <header>
    <h2 id="drawer-title">Folders</h2>
    <button class="icon-btn" type="button" aria-label="Close folders" onclick={onclose}><X size={20} /></button>
  </header>

  <div class="body">
    {#if first}
      <p class="intro">Pick the folders you want to search. rummage indexes them in the background; results show up as it goes.</p>
    {/if}

    {#if fromEnv}
      <p class="note">These folders come from the <code>RUMMAGE_FOLDERS</code> variable, so they can't be changed here.</p>
    {:else}
      <form
        class="add"
        onsubmit={(e) => {
          e.preventDefault();
          if (path.trim()) add(path);
        }}
      >
        <label class="sr-only" for="folder-path">Folder path</label>
        <input id="folder-path" class="input" bind:this={pathInput} bind:value={path} placeholder="Paste a folder path, e.g. /home/you/Pictures" autocomplete="off" spellcheck="false" />
        <button class="btn primary" type="submit" disabled={!path.trim() || adding}>Add</button>
        <button class="btn" type="button" aria-expanded={browsing} onclick={() => (browsing = !browsing)}><FolderTree size={16} />Browse</button>
      </form>
      {#if addError}<p class="err" role="alert">{addError}</p>{/if}
      {#if browsing}
        <DirBrowser onadd={add} />
      {/if}
    {/if}

    {#if app.folders.length && s}
      <div class="global" class:paused>
        <span class="dot" class:live={!paused && (s.state !== 'idle' || pending > 0)} aria-hidden="true"></span>
        <span class="what">
          {#if paused}
            Indexing is paused{#if pending}; {plural(pending, 'file')} waiting{/if}. Search still works.
          {:else if s.state === 'scanning' && !pending}
            Looking for new and changed files…
          {:else if pending}
            Indexing {plural(pending, 'file')}{#if s.eta_secs}{` · ${eta(s.eta_secs)} left`}{/if}
          {:else}
            Everything is indexed.
          {/if}
        </span>
        <button class="btn sm" type="button" onclick={() => app.setPaused(!paused)}>
          {#if paused}<Play size={14} />Resume indexing{:else}<Pause size={14} />Pause indexing{/if}
        </button>
      </div>
    {/if}

    {#if app.folders.length}
      <ul class="folders">
        {#each app.folders as f (f.id)}
          {@const fp = progress(f)}
          {@const c = fp.counts}
          {@const pct = c.files ? (100 * c.indexed) / c.files : 0}
          <li class="folder">
            <div class="row">
              <div class="name">
                <strong title={f.path}>{base(f.path)}</strong>
                <span class="parent" title={f.path}>{parent(f.path)}</span>
              </div>
              <span class="state">
                {#if !f.available}
                  <span class="tag bad"><TriangleAlert size={12} />Not found</span>
                {:else if !f.enabled}
                  <span class="tag">Paused</span>
                {:else if fp.scanning}
                  <span class="tag live">Scanning</span>
                {:else if c.pending > 0 && paused}
                  <span class="tag">Waiting</span>
                {:else if c.pending > 0}
                  <span class="tag live">Indexing</span>
                {:else}
                  <span class="tag ok">Up to date</span>
                {/if}
              </span>
            </div>
            <div class="progress" role="progressbar" aria-label="{base(f.path)} indexed" aria-valuenow={Math.round(pct)} aria-valuemin="0" aria-valuemax="100">
              <span style:width="{pct}%"></span>
            </div>
            <p class="meta num">
              {num(c.indexed)} of {plural(c.files, 'file')} indexed
              {#if fp.scanning}· looking for new and changed files{/if}
              {#if c.pending > 0 && f.enabled && !paused && fp.eta_secs}· {eta(fp.eta_secs)} left{/if}
              {#if c.errors || c.skipped}·
                <button class="link warn" type="button" onclick={showProblems}>
                  {#if c.errors}{num(c.errors)} could not be read{/if}{#if c.errors && c.skipped}{', '}{/if}{#if c.skipped}{num(c.skipped)} skipped{/if}
                </button>
              {/if}
              {#if f.exclude.length}· {plural(f.exclude.length, 'exclusion')}{/if}
            </p>
            {#if !f.from_env}
              <div class="acts">
                <button class="btn sm ghost" type="button" title={f.enabled ? 'Stop indexing and watching this folder' : 'Index and watch this folder again'} onclick={() => act(f, () => api.updateFolder(f.id, { enabled: !f.enabled }), f.enabled ? 'Paused' : 'Resumed')}>
                  {#if f.enabled}<Pause size={14} />Pause{:else}<Play size={14} />Resume{/if}
                </button>
                <button class="btn sm ghost" type="button" onclick={() => act(f, () => api.reindex({ scope: 'folder', id: f.id }), 'Reindexing')}><RefreshCw size={14} />Reindex</button>
                <button class="btn sm ghost" type="button" aria-expanded={editing === f.id} onclick={() => edit(f)}><SlidersHorizontal size={14} />Exclude…</button>
                {#if confirming === f.id}
                  <span class="confirm">
                    Remove from search? Files stay on disk.
                    <button class="btn sm danger" type="button" onclick={() => act(f, () => api.removeFolder(f.id), 'Folder removed').then(() => (confirming = null))}>Remove</button>
                    <button class="btn sm ghost" type="button" onclick={() => (confirming = null)}>Cancel</button>
                  </span>
                {:else}
                  <button class="btn sm ghost danger" type="button" onclick={() => (confirming = f.id)}><Trash2 size={14} />Remove</button>
                {/if}
              </div>
            {/if}
            {#if editing === f.id}
              <div class="excl">
                <label for="excl-{f.id}">Skip files matching these patterns, one per line (relative to the folder)</label>
                <textarea id="excl-{f.id}" class="input" rows="4" bind:value={excludes} placeholder={'**/*.raw\nScreenshots/**'}></textarea>
                <div class="excl-acts">
                  <button class="btn sm primary" type="button" onclick={() => saveExcludes(f)}>Save</button>
                  <button class="btn sm ghost" type="button" onclick={() => (editing = null)}>Cancel</button>
                </div>
              </div>
            {/if}
            {#if rowError[f.id]}<p class="err" role="alert">{rowError[f.id]}</p>{/if}
          </li>
        {/each}
      </ul>
      {#if !fromEnv}
        <button class="btn sm" type="button" onclick={() => api.reindex({ scope: 'all' }).then(() => app.say('Reindexing everything'))}><RefreshCw size={14} />Reindex everything</button>
      {/if}
    {/if}

    {#if problems}
      <section class="problems" aria-labelledby="problems-title">
        <div class="sec-head">
          <h3 id="problems-title" tabindex="-1" bind:this={problemsTitle}>Not indexed <span class="count num">{num(problems.total)}</span></h3>
          <button class="btn sm" type="button" onclick={() => app.retryProblems()}><RotateCcw size={14} />Try again</button>
        </div>
        <ul>
          {#each problems.items as it (it.id)}
            <li>
              <div class="pname">
                <strong title={it.path}>{it.name}</strong>
                <span class="tag" class:bad={it.status === 'error'}>{it.status === 'skipped' ? 'Skipped' : "Can't read"}</span>
              </div>
              <span class="parent" title={it.path}>{dirname(it.path)}</span>
              {#if it.error}<span class="why" title={it.error}>{it.error}</span>{/if}
            </li>
          {/each}
        </ul>
        {#if problems.total > problems.items.length}
          <p class="hint">The newest {num(problems.items.length)} of {num(problems.total)}.</p>
        {/if}
      </section>
    {/if}

    <section class="settings">
      <h3>Open in Files</h3>
      <p class="hint">A link template for a file browser. <code>{'{path}'}</code> is the file's path, <code>{'{dir}'}</code> its folder. Leave empty to hide the button.</p>
      <form
        class="add"
        onsubmit={(e) => {
          e.preventDefault();
          saveFilesUrl();
        }}
      >
        <label class="sr-only" for="files-url">Open in Files link</label>
        <input id="files-url" class="input" bind:value={filesUrl} placeholder="/files/files{'{path}'}" spellcheck="false" />
        <button class="btn" type="submit">{savedFilesUrl ? 'Saved' : 'Save'}</button>
      </form>
    </section>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--scrim);
    animation: fade var(--med) var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .drawer {
    position: fixed;
    z-index: 51;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(480px, 100vw);
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow-lg);
    animation: in var(--med) var(--ease);
  }
  @keyframes in {
    from {
      transform: translateX(32px);
      opacity: 0;
    }
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 32px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .intro {
    margin: 0;
    font-size: 14.5px;
    color: var(--text-2);
  }
  .note,
  .hint {
    margin: 0;
    font-size: 13px;
    color: var(--muted);
  }
  code {
    font: 12.5px var(--mono);
  }
  .add {
    display: flex;
    gap: 8px;
  }
  .add .input {
    flex: 1;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13.5px;
  }
  .folders {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .folder {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    padding: 12px 14px 8px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }
  .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name strong {
    font-weight: 600;
    font-size: 14.5px;
  }
  .parent {
    font-size: 12.5px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-2);
    white-space: nowrap;
  }
  .tag.ok {
    color: var(--ok);
  }
  .tag.live {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .tag.bad {
    color: var(--danger);
  }
  .progress {
    height: 4px;
    margin: 10px 0 6px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--med) var(--ease);
  }
  .meta {
    margin: 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .warn {
    color: var(--warn);
  }
  .global {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: var(--surface-2);
    font-size: 13.5px;
    color: var(--text-2);
  }
  .global .what {
    flex: 1;
  }
  .global .btn {
    flex: none;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--ok);
    flex: none;
  }
  .paused .dot {
    background: var(--muted);
  }
  .dot.live {
    background: var(--accent);
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .problems {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .sec-head h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    outline: none;
    scroll-margin-top: 16px;
  }
  .count {
    color: var(--muted);
    font-weight: 500;
  }
  .problems ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .problems li {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 8px 12px;
    min-width: 0;
  }
  .problems li + li {
    border-top: 1px solid var(--border);
  }
  .pname {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }
  .pname strong {
    font-weight: 600;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .why {
    font-size: 12.5px;
    color: var(--text-2);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    margin: 6px -8px 0;
  }
  .confirm {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text-2);
    margin-left: 6px;
  }
  .excl {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 8px 0 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  textarea.input {
    height: auto;
    padding: 8px 10px;
    font: 13px var(--mono);
    resize: vertical;
  }
  .excl-acts {
    display: flex;
    gap: 6px;
  }
  .settings {
    margin-top: 10px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .settings h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
</style>
