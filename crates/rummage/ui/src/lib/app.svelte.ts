// The page's state and everything that changes it: the search, the open preview, the index
// status and the folders. Components read from `app` and call its methods.
import { tick } from 'svelte';
import {
  api,
  ApiError,
  isAbort,
  type Folder,
  type IndexStatus,
  type Item,
  type ItemDetail,
  type Kind,
  type ProblemList,
  type SearchHit,
  type SearchRequest,
  type SearchResponse,
  type Segment,
} from './api';
import { loadRecent, pushRecent, saveRecent } from './recent';
import { dateRange, empty, parse, searchKey, serialize, type UrlState } from './urlstate';

export const PAGE = 60;
const DEBOUNCE_MS = 150;

export type Preview = {
  item: Item | null;
  /** The matched segment to open at. */
  segment: Segment | null;
  detail: ItemDetail | null;
  error: string;
};

type KindCounts = Partial<Record<Kind, number>>;

/** Escapes glob metacharacters so a directory name matches literally. */
function globEscape(s: string) {
  return s.replace(/[*?[\]{}\\!]/g, '\\$&');
}

class App {
  url = $state<UrlState>(empty());
  hits = $state.raw<SearchHit[]>([]);
  kinds = $state.raw<KindCounts>({});
  loading = $state(false);
  loadingMore = $state(false);
  /** No more pages to load. */
  done = $state(true);
  error = $state<ApiError | null>(null);
  /** Search-by-file in progress (not representable in the URL). */
  upload = $state<{ file: File; thumb: string } | null>(null);
  /** The item "More like this" searches by, for its label. */
  likeItem = $state<Item | null>(null);
  selected = $state(0);
  preview = $state<Preview | null>(null);

  status = $state<IndexStatus | null>(null);
  folders = $state<Folder[]>([]);
  foldersLoaded = $state(false);
  /** Files that failed to index or were skipped, newest first. */
  problems = $state.raw<ProblemList | null>(null);
  drawerOpen = $state(false);
  /** The drawer section to scroll to when it opens. */
  drawerFocus = $state<'' | 'problems'>('');
  filesUrl = $state('');
  recent = $state<string[]>(loadRecent());
  needToken = $state(false);
  offline = $state(false);
  toast = $state('');

  /** Where focus goes back to when the preview closes. */
  returnFocus: HTMLElement | null = null;
  private ctrl: AbortController | null = null;
  private moreCtrl: AbortController | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private recentTimer: ReturnType<typeof setTimeout> | undefined;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  private detailCtrl: AbortController | null = null;
  private stopEvents: (() => void) | null = null;
  private problemsTimer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;
  private searching: Promise<void> | null = null;

  /** True when there is something to show results for. */
  get active() {
    return !!(this.url.q.trim() || this.url.like != null || this.upload);
  }

  get filtered() {
    const u = this.url;
    return !!(u.type || u.folder || u.date || u.dir);
  }

  // ---- start-up ----

  async init() {
    this.url = parse(location.search);
    window.addEventListener('popstate', () => this.onPopState());
    if (this.url.like != null) this.loadLikeItem(this.url.like);
    if (this.active) this.runSearch();
    if (this.url.item != null) this.openId(this.url.item);
    await this.loadIndex();
  }

  async loadIndex() {
    try {
      const health = await api.health();
      this.offline = false;
      if (health.auth_required && !(await this.tokenWorks())) {
        this.needToken = true;
        return;
      }
    } catch (e) {
      this.offline = true;
      return;
    }
    this.needToken = false;
    const [status, folders, settings] = await Promise.allSettled([api.status(), api.folders(), api.settings()]);
    if (status.status === 'fulfilled') this.setStatus(status.value);
    if (folders.status === 'fulfilled') {
      this.folders = folders.value;
      this.foldersLoaded = true;
      if (!this.folders.length) this.drawerOpen = true;
    }
    if (settings.status === 'fulfilled') this.filesUrl = settings.value.settings.ui?.files_url ?? '';
    this.stopEvents?.();
    this.stopEvents = api.events((e) => {
      if (e.type === 'status') {
        const { type: _, ...s } = e;
        this.setStatus(s);
      } else if (e.status === 'error' || e.status === 'skipped') {
        this.refreshProblemsSoon();
      }
    });
  }

  /** Takes a status snapshot; the problem list follows when its counts change. */
  private setStatus(s: IndexStatus) {
    const was = this.status ? this.status.totals.errors + (this.status.totals.skipped ?? 0) : -1;
    this.status = s;
    const now = s.totals.errors + (s.totals.skipped ?? 0);
    if (now !== was) {
      if (now === 0) this.problems = { total: 0, items: [] };
      else this.refreshProblemsSoon();
    }
  }

  private refreshProblemsSoon() {
    clearTimeout(this.problemsTimer);
    this.problemsTimer = setTimeout(() => this.refreshProblems(), 300);
  }

  async refreshProblems() {
    clearTimeout(this.problemsTimer);
    try {
      this.problems = await api.problems();
    } catch {
      /* the status line still shows the counts */
    }
  }

  /** Pauses or resumes all indexing; searches keep working either way. */
  async setPaused(paused: boolean) {
    try {
      this.setStatus(await (paused ? api.pause() : api.resume()));
      this.say(paused ? 'Indexing paused' : 'Indexing resumed');
    } catch (e) {
      this.say(e instanceof ApiError ? e.message : String(e));
    }
  }

  /** Queues every failed or skipped file for another try. */
  async retryProblems() {
    try {
      const r = await api.reindex({ scope: 'problems' });
      this.say(r.queued ? `Trying ${r.queued === 1 ? '1 file' : `${r.queued} files`} again` : 'Nothing to retry');
    } catch (e) {
      this.say(e instanceof ApiError ? e.message : String(e));
    }
  }

  private async tokenWorks() {
    try {
      await api.status();
      return true;
    } catch (e) {
      return !(e instanceof ApiError && e.status === 401);
    }
  }

  async refreshFolders() {
    try {
      this.folders = await api.folders();
      this.foldersLoaded = true;
    } catch (e) {
      this.handleError(e);
    }
  }

  // ---- URL and history ----

  /** Writes the state to the URL. `state.preview` marks entries that opened the preview. */
  private write(mode: 'push' | 'replace', state: object = {}) {
    const u = serialize(this.url);
    const same = u === location.search || (u === location.pathname && !location.search);
    if (mode === 'push' && !same) history.pushState(state, '', u);
    else history.replaceState(state, '', u);
  }

  private onPopState() {
    const next = parse(location.search);
    const changed = searchKey(next) !== searchKey(this.url);
    this.url = next;
    if (changed) {
      this.upload = null;
      if (next.like != null) this.loadLikeItem(next.like);
      if (this.active) this.runSearch();
      else this.clearResults();
    }
    if (next.item == null) this.closePreviewState();
    else if (this.preview?.item?.id !== next.item) this.openId(next.item);
  }

  // ---- search ----

  setQuery(q: string) {
    const wasActive = this.active;
    this.url.q = q;
    this.url.like = null;
    this.likeItem = null;
    this.clearUpload();
    if (this.preview) this.closePreviewState();
    this.url.item = null;
    this.write(!wasActive && this.active ? 'push' : 'replace');
    clearTimeout(this.timer);
    if (!this.active) {
      this.ctrl?.abort();
      this.clearResults();
      return;
    }
    // Counts as loading from the keystroke on, so "No matches" never flashes before the search.
    this.loading = true;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.runSearch();
    }, DEBOUNCE_MS);
  }

  /** Waits for the results of what is typed, starting the search now if it is still debounced. */
  async settle() {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
      if (this.active) await this.runSearch();
    } else if (this.searching) {
      await this.searching;
    }
  }

  setFilter(patch: Partial<UrlState>) {
    Object.assign(this.url, patch);
    this.url.item = null;
    this.closePreviewState();
    this.write('push');
    if (this.active) this.runSearch();
  }

  clearFilters() {
    this.setFilter({ type: '', folder: '', date: '', from: '', to: '', dir: '' });
  }

  /** Back to the empty page. */
  home() {
    this.ctrl?.abort();
    this.clearUpload();
    this.likeItem = null;
    this.url = empty();
    this.closePreviewState();
    this.clearResults();
    this.write('push');
  }

  moreLike(item: Item) {
    this.clearUpload();
    this.likeItem = item;
    this.url = { ...empty(), like: item.id };
    this.closePreviewState();
    this.write('push');
    this.runSearch();
  }

  searchByFile(file: File) {
    this.clearUpload();
    this.upload = { file, thumb: file.type.startsWith('image/') ? URL.createObjectURL(file) : '' };
    this.likeItem = null;
    this.url = { ...empty(), type: this.url.type };
    this.closePreviewState();
    this.write('push');
    this.runSearch();
  }

  clearUpload() {
    if (this.upload?.thumb) URL.revokeObjectURL(this.upload.thumb);
    this.upload = null;
  }

  private clearResults() {
    this.hits = [];
    this.kinds = {};
    this.error = null;
    this.loading = false;
    this.done = true;
    this.selected = 0;
  }

  private async loadLikeItem(id: number) {
    if (this.likeItem?.id === id) return;
    try {
      this.likeItem = (await api.item(id)).item;
    } catch {
      this.likeItem = null;
    }
  }

  request(offset: number): SearchRequest {
    const u = this.url;
    const range = dateRange(u);
    return {
      query: u.like == null && !this.upload ? u.q.trim() : null,
      item_id: u.like,
      file: null,
      mode: 'hybrid',
      limit: PAGE,
      offset,
      filters: {
        types: u.type ? [u.type] : [],
        folders: u.folder ? [u.folder] : [],
        date_from: range.date_from ?? null,
        date_to: range.date_to ?? null,
        path_glob: u.dir ? `${globEscape(u.dir.replace(/\/+$/, ''))}/**` : null,
      },
    };
  }

  private send(req: SearchRequest, signal: AbortSignal): Promise<SearchResponse> {
    if (this.upload) return api.searchFile(this.upload.file, { ...req, query: null, item_id: null }, signal);
    return api.search(req, signal);
  }

  runSearch(): Promise<void> {
    const p = this.doSearch();
    this.searching = p;
    return p;
  }

  private async doSearch() {
    this.ctrl?.abort();
    this.moreCtrl?.abort();
    this.loadingMore = false;
    const ctrl = (this.ctrl = new AbortController());
    const seq = ++this.seq;
    this.loading = true;
    const req = this.request(0);
    try {
      const res = await this.send(req, ctrl.signal);
      if (seq !== this.seq) return;
      this.hits = res.hits;
      this.kinds = Object.fromEntries((res.kinds ?? []).map((k) => [k.kind, k.files]));
      this.done = res.hits.length < req.limit!;
      this.selected = 0;
      this.error = null;
      this.offline = false;
      this.loading = false;
      await tick();
      requestAnimationFrame(() => performance.mark('rummage:results'));
      this.rememberSoon();
    } catch (e) {
      if (isAbort(e) || seq !== this.seq) return;
      this.loading = false;
      this.hits = [];
      this.kinds = {};
      this.handleError(e);
    }
  }

  async loadMore() {
    if (this.loading || this.loadingMore || this.done || !this.active) return;
    const seq = this.seq;
    const ctrl = (this.moreCtrl = new AbortController());
    this.loadingMore = true;
    const req = this.request(this.hits.length);
    try {
      const res = await this.send(req, ctrl.signal);
      if (seq !== this.seq) return;
      const seen = new Set(this.hits.map((h) => h.item.id));
      this.hits = [...this.hits, ...res.hits.filter((h) => !seen.has(h.item.id))];
      this.done = res.hits.length < req.limit!;
    } catch (e) {
      if (!isAbort(e)) this.handleError(e);
    } finally {
      if (seq === this.seq) this.loadingMore = false;
    }
  }

  private handleError(e: unknown) {
    if (!(e instanceof ApiError)) {
      this.error = new ApiError('internal', String(e));
      return;
    }
    if (e.status === 401) this.needToken = true;
    if (e.code === 'network') this.offline = true;
    this.error = e;
  }

  private rememberSoon() {
    clearTimeout(this.recentTimer);
    this.recentTimer = setTimeout(() => this.remember(), 1500);
  }

  remember() {
    clearTimeout(this.recentTimer);
    const q = this.url.q.trim();
    if (!q || this.url.like != null) return;
    this.recent = pushRecent(this.recent, q);
    saveRecent(this.recent);
  }

  forgetRecent() {
    this.recent = [];
    saveRecent([]);
  }

  // ---- preview ----

  openIndex(i: number, how: 'push' | 'replace' = 'push') {
    const hit = this.hits[i];
    if (!hit) return;
    this.selected = i;
    this.remember();
    const wasOpen = !!this.preview;
    this.url.item = hit.item.id;
    this.write(how === 'push' && !wasOpen ? 'push' : 'replace', { preview: true });
    this.preview = { item: hit.item, segment: hit.segment ?? null, detail: null, error: '' };
    this.loadDetail(hit.item.id);
  }

  /** Opens an item by id (deep links, Back/Forward). */
  openId(id: number) {
    const i = this.hits.findIndex((h) => h.item.id === id);
    if (i >= 0) {
      this.selected = i;
      const hit = this.hits[i];
      this.preview = { item: hit.item, segment: hit.segment ?? null, detail: null, error: '' };
    } else {
      this.preview = { item: null, segment: null, detail: null, error: '' };
    }
    this.url.item = id;
    this.loadDetail(id);
  }

  private async loadDetail(id: number) {
    this.detailCtrl?.abort();
    const ctrl = (this.detailCtrl = new AbortController());
    try {
      const q = this.url.like == null ? this.url.q.trim() : '';
      const detail = await api.item(id, q || undefined, ctrl.signal);
      const p = this.preview;
      if (!p || this.url.item !== id) return;
      this.preview = {
        ...p,
        item: detail.item,
        detail,
        segment: p.segment ?? (q ? (detail.segments[0] ?? null) : null),
      };
    } catch (e) {
      if (isAbort(e)) return;
      const p = this.preview;
      if (!p || this.url.item !== id) return;
      const msg =
        e instanceof ApiError && e.code === 'not_found'
          ? 'This item is no longer in the index. The file may have been moved or deleted.'
          : e instanceof ApiError
            ? e.message
            : String(e);
      this.preview = { ...p, error: p.item ? '' : msg };
      if (e instanceof ApiError && e.status === 401) this.needToken = true;
    }
  }

  /** Moves the open preview to the next (+1) or previous (-1) result. */
  async step(delta: 1 | -1) {
    if (!this.preview?.item) return;
    let i = this.hits.findIndex((h) => h.item.id === this.preview!.item!.id);
    if (i < 0) i = this.selected - delta;
    const next = i + delta;
    if (next >= this.hits.length && !this.done) await this.loadMore();
    if (next < 0 || next >= this.hits.length) return;
    this.openIndex(next, 'replace');
  }

  closePreview() {
    if (!this.preview) return;
    if (history.state?.preview) {
      history.back();
      this.closePreviewState();
      return;
    }
    this.closePreviewState();
    this.url.item = null;
    this.write('replace', {});
  }

  private closePreviewState() {
    this.detailCtrl?.abort();
    if (!this.preview) return;
    this.preview = null;
    const el = this.returnFocus;
    this.returnFocus = null;
    if (el?.isConnected) tick().then(() => el.focus({ preventScroll: false }));
  }

  // ---- folders ----

  openDrawer(focus: '' | 'problems' = '') {
    this.drawerFocus = focus;
    this.drawerOpen = true;
    if (!this.foldersLoaded) this.refreshFolders();
    if (focus === 'problems' || !this.problems) this.refreshProblems();
  }

  // ---- misc ----

  say(msg: string) {
    this.toast = msg;
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = ''), 2400);
  }
}

export const app = new App();
