//! The HTTP server: `/api/v1`, the OpenAPI description, and the embedded web UI.

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Extension, Json, Router};
use futures_util::{Stream, StreamExt, stream};
use rummage_core::service::{self, Service};
use rummage_core::types::*;
use rummage_core::{Config, config::normalize_base_path};
use tokio_stream::wrappers::BroadcastStream;
use tower::{Layer, ServiceExt};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::{OpenApi as OpenApiDoc, Server};
use utoipa::{Modify, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Largest request body accepted (search-by-file uploads).
const MAX_UPLOAD: usize = 64 * 1024 * 1024;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "rummage",
        description = "Local multimodal semantic search. Everything the web UI does goes through this API.\n\n\
            If `server.auth_token` is set, send `Authorization: Bearer <token>` (or `?access_token=<token>` \
            where headers can't be set, e.g. `<video src>`). `/api/v1/health` never needs a token.\n\n\
            Times are Unix seconds. Media offsets are seconds from the start."
    ),
    tags(
        (name = "health", description = "Liveness and version"),
        (name = "folders", description = "Which folders are indexed"),
        (name = "index", description = "Indexing progress and re-indexing"),
        (name = "search", description = "Search by text, by item or by file"),
        (name = "items", description = "Indexed files: metadata, thumbnails, previews and the file itself"),
        (name = "settings", description = "The config file's keys, over HTTP"),
    ),
    modifiers(&BearerAuth)
)]
struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut OpenApiDoc) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
        openapi.security = Some(vec![
            Default::default(),
            utoipa::openapi::security::SecurityRequirement::new("bearer", Vec::<String>::new()),
        ]);
    }
}

pub struct AppState {
    pub svc: Arc<Service>,
    openapi: OpenApiDoc,
}

/// The URL prefix the client sees (`""` or e.g. `/search`), for links in responses.
#[derive(Clone, Debug, Default)]
pub struct Prefix(pub String);

/// The service type of the whole app.
pub type App = tower::util::BoxCloneSyncService<Request, Response, Infallible>;

/// Builds the full app: the base-path handling wraps the router, so it sees the original URL.
pub fn app(svc: Arc<Service>) -> App {
    let base: Arc<str> = svc.config().base_path().into();
    let router = router(svc);
    App::new(middleware::from_fn_with_state(base, base_path).layer(router))
}

/// The routes, without base-path handling.
pub fn router(svc: Arc<Service>) -> Router {
    let (api, openapi) = api_router().split_for_parts();
    let state = Arc::new(AppState { svc, openapi });
    Router::new()
        .merge(api)
        .route("/api/openapi.json", axum::routing::get(openapi_json))
        .route("/api/docs", axum::routing::get(docs))
        .route(
            "/api",
            axum::routing::get(|| async { Redirect::to("api/docs") }),
        )
        .fallback(ui)
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(state)
}

/// The `/api/v1` routes with their OpenAPI description.
pub fn api_router() -> OpenApiRouter<Arc<AppState>> {
    let v1 = OpenApiRouter::new()
        .routes(routes!(health))
        .routes(routes!(list_folders, add_folder))
        .routes(routes!(get_folder, update_folder, remove_folder))
        .routes(routes!(index_status))
        .routes(routes!(index_events))
        .routes(routes!(reindex))
        .routes(routes!(search))
        .routes(routes!(get_item))
        .routes(routes!(item_thumb))
        .routes(routes!(item_preview))
        .routes(routes!(item_raw))
        .routes(routes!(item_similar))
        .routes(routes!(get_settings, patch_settings));
    OpenApiRouter::with_openapi(ApiDoc::openapi()).nest("/api/v1", v1)
}

/// The OpenAPI description, as served (without a `servers` entry).
pub fn openapi() -> OpenApiDoc {
    api_router().split_for_parts().1
}

// ---- base path, prefix and auth ----

/// Strips the configured base path (if the proxy kept it) and records the client-facing prefix
/// from the base path or `X-Forwarded-Prefix` (if the proxy stripped it).
async fn base_path(State(base): State<Arc<str>>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let mut prefix = String::new();
    if !base.is_empty() {
        if path == *base {
            let q = req
                .uri()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default();
            return Redirect::permanent(&format!("{base}/{q}")).into_response();
        }
        if let Some(rest) = path.strip_prefix(&*base).filter(|r| r.starts_with('/')) {
            let pq = match req.uri().query() {
                Some(q) => format!("{rest}?{q}"),
                None => rest.to_string(),
            };
            let mut parts = req.uri().clone().into_parts();
            parts.path_and_query = pq.parse().ok();
            if let Ok(uri) = Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
            prefix = base.to_string();
        }
    }
    if let Some(fwd) = req
        .headers()
        .get("x-forwarded-prefix")
        .and_then(|v| v.to_str().ok())
        .map(normalize_base_path)
        .filter(|p| {
            p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/-_.~".contains(c))
        })
    {
        prefix = fwd;
    }
    req.extensions_mut().insert(Prefix(prefix));
    next.run(req).await
}

async fn auth(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/v1/") || path == "/api/v1/health" {
        return next.run(req).await;
    }
    let token = st.svc.config().server.auth_token;
    if token.is_empty() {
        return next.run(req).await;
    }
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let query = req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("access_token="))
            .map(str::to_string)
    });
    if bearer.or(query).is_some_and(|t| constant_eq(&t, &token)) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            Json(ApiError {
                error: "unauthorized".into(),
                message: "missing or wrong bearer token".into(),
            }),
        )
            .into_response()
    }
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

// ---- errors ----

/// A service error as an HTTP response with an [`ApiError`] body.
pub struct ApiErr(pub service::Error);

impl From<service::Error> for ApiErr {
    fn from(e: service::Error) -> Self {
        ApiErr(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        use service::Error as E;
        let status = match &self.0 {
            E::BadRequest(_) => StatusCode::BAD_REQUEST,
            E::NotFound(_) => StatusCode::NOT_FOUND,
            E::Conflict(_) | E::NotReady(_) => StatusCode::CONFLICT,
            E::Unsupported(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            E::Internal(e) => {
                tracing::error!("{e:#}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            Json(ApiError {
                error: self.0.code().into(),
                message: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiErr>;

/// Runs blocking store/model work off the async threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> service::Result<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| service::Error::Internal(e.into()))?
        .map_err(ApiErr)
}

// ---- handlers ----

/// Liveness, version and the embedding model in use. Never needs a token.
#[utoipa::path(get, path = "/health", tag = "health",
    responses((status = 200, body = Health)))]
async fn health(State(st): State<Arc<AppState>>) -> ApiResult<Json<Health>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.health()).await?))
}

/// The indexed folders with their file counts.
#[utoipa::path(get, path = "/folders", tag = "folders",
    responses((status = 200, body = Vec<Folder>)))]
async fn list_folders(State(st): State<Arc<AppState>>) -> ApiResult<Json<Vec<Folder>>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.folders()).await?))
}

/// Adds a folder to the config file. Indexing starts on its own.
#[utoipa::path(post, path = "/folders", tag = "folders", request_body = NewFolder,
    responses(
        (status = 201, body = Folder),
        (status = 400, body = ApiError, description = "Not an absolute path to a directory, or a bad glob"),
        (status = 409, body = ApiError, description = "Already watched, or folders come from RUMMAGE_FOLDERS"),
    ))]
async fn add_folder(
    State(st): State<Arc<AppState>>,
    Json(body): Json<NewFolder>,
) -> ApiResult<(StatusCode, Json<Folder>)> {
    let svc = st.svc.clone();
    let f = blocking(move || svc.add_folder(body)).await?;
    Ok((StatusCode::CREATED, Json(f)))
}

/// One folder.
#[utoipa::path(get, path = "/folders/{id}", tag = "folders",
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 200, body = Folder), (status = 404, body = ApiError)))]
async fn get_folder(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Folder>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.folder(&id)).await?))
}

/// Changes a folder's globs or turns it on or off.
#[utoipa::path(patch, path = "/folders/{id}", tag = "folders", request_body = FolderPatch,
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 200, body = Folder), (status = 400, body = ApiError), (status = 404, body = ApiError)))]
async fn update_folder(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<FolderPatch>,
) -> ApiResult<Json<Folder>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.update_folder(&id, body)).await?))
}

/// Stops watching a folder and removes its files from the index (the files themselves stay).
#[utoipa::path(delete, path = "/folders/{id}", tag = "folders",
    params(("id" = String, Path, description = "Folder id")),
    responses((status = 204), (status = 404, body = ApiError)))]
async fn remove_folder(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let svc = st.svc.clone();
    blocking(move || svc.remove_folder(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where indexing stands: totals, per kind, per folder, and the queue.
#[utoipa::path(get, path = "/index/status", tag = "index",
    responses((status = 200, body = IndexStatus)))]
async fn index_status(State(st): State<Arc<AppState>>) -> ApiResult<Json<IndexStatus>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.index_status()).await?))
}

/// Live progress as server-sent events. The first event is a `status` snapshot; then `status`
/// and `file` events as indexing moves.
#[utoipa::path(get, path = "/index/events", tag = "index",
    responses((status = 200, content_type = "text/event-stream", body = IndexEvent,
        description = "`event: status|file`, `data:` one IndexEvent as JSON")))]
async fn index_events(
    State(st): State<Arc<AppState>>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let rx = st.svc.subscribe();
    let svc = st.svc.clone();
    let first = IndexEvent::Status(blocking(move || svc.index_status()).await?);
    let events = stream::once(async move { first })
        .chain(BroadcastStream::new(rx).filter_map(|e| async move { e.ok() }))
        .map(|e| {
            Ok(Event::default()
                .event(e.name())
                .json_data(&e)
                .unwrap_or_default())
        });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Queues everything, one folder, or one item for re-indexing.
#[utoipa::path(post, path = "/reindex", tag = "index", request_body = ReindexRequest,
    responses((status = 202, body = ReindexResponse), (status = 404, body = ApiError)))]
async fn reindex(
    State(st): State<Arc<AppState>>,
    Json(body): Json<ReindexRequest>,
) -> ApiResult<(StatusCode, Json<ReindexResponse>)> {
    let svc = st.svc.clone();
    let r = blocking(move || svc.reindex(&body)).await?;
    Ok((StatusCode::ACCEPTED, Json(r)))
}

/// The multipart form of `POST /search`.
#[derive(ToSchema)]
#[allow(dead_code)]
struct SearchUpload {
    /// The file to search by (an image or a text file).
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
    /// Filters, mode, limit and offset, as a SearchRequest JSON string.
    request: Option<SearchRequest>,
}

/// Searches by text, by an indexed item, or by an uploaded file, with optional filters.
#[utoipa::path(post, path = "/search", tag = "search",
    request_body(content(
        (SearchRequest = "application/json"),
        (SearchUpload = "multipart/form-data"),
    )),
    responses(
        (status = 200, body = SearchResponse),
        (status = 400, body = ApiError),
        (status = 409, body = ApiError, description = "The item to search by is not indexed yet"),
        (status = 415, body = ApiError, description = "The uploaded file type can't be searched by"),
    ))]
async fn search(
    State(st): State<Arc<AppState>>,
    Extension(Prefix(prefix)): Extension<Prefix>,
    req: Request,
) -> ApiResult<Json<SearchResponse>> {
    let svc = st.svc.clone();
    let is_multipart = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    let bad = |m: String| ApiErr(service::Error::BadRequest(m));
    if is_multipart {
        let mut form = Multipart::from_request(req, &())
            .await
            .map_err(|e| bad(e.body_text()))?;
        let (mut request, mut file) = (SearchRequest::default(), None);
        while let Some(field) = form.next_field().await.map_err(|e| bad(e.body_text()))? {
            match field.name() {
                Some("request") => {
                    let text = field.text().await.map_err(|e| bad(e.body_text()))?;
                    request =
                        serde_json::from_str(&text).map_err(|e| bad(format!("request: {e}")))?;
                }
                Some("file") => {
                    let name = field.file_name().map(String::from);
                    let mime = field.content_type().map(String::from);
                    let bytes = field.bytes().await.map_err(|e| bad(e.body_text()))?;
                    file = Some((name, mime, bytes.to_vec()));
                }
                _ => {}
            }
        }
        let (name, mime, bytes) = file.ok_or_else(|| bad("missing `file` part".into()))?;
        Ok(Json(
            blocking(move || svc.search_upload(request, name, mime, bytes, &prefix)).await?,
        ))
    } else {
        let Json(body) = Json::<SearchRequest>::from_request(req, &())
            .await
            .map_err(|e| bad(e.body_text()))?;
        Ok(Json(blocking(move || svc.search(body, &prefix)).await?))
    }
}

/// An item's metadata and segments (passages, pages, time windows).
#[utoipa::path(get, path = "/items/{id}", tag = "items",
    params(("id" = i64, Path, description = "Item id"), ItemParams),
    responses((status = 200, body = ItemDetail), (status = 404, body = ApiError)))]
async fn get_item(
    State(st): State<Arc<AppState>>,
    Extension(Prefix(prefix)): Extension<Prefix>,
    Path(id): Path<i64>,
    Query(p): Query<ItemParams>,
) -> ApiResult<Json<ItemDetail>> {
    let svc = st.svc.clone();
    Ok(Json(
        blocking(move || svc.item(id, p.q.as_deref(), &prefix)).await?,
    ))
}

/// A small JPEG of the item.
#[utoipa::path(get, path = "/items/{id}/thumb", tag = "items",
    params(("id" = i64, Path, description = "Item id"), ThumbParams),
    responses(
        (status = 200, content_type = "image/jpeg", body = Vec<u8>),
        (status = 404, body = ApiError, description = "No such item, or no thumbnail yet"),
    ))]
async fn item_thumb(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(p): Query<ThumbParams>,
    req: Request,
) -> ApiResult<Response> {
    let size = p.size.unwrap_or(st.svc.config().index.thumbnail_size);
    let cached = st.svc.cache_path("thumbs", id, &size.to_string());
    cached_or_image(&st, id, cached, req).await
}

/// A rendered view of the item: a document page or a video frame (images return themselves).
#[utoipa::path(get, path = "/items/{id}/preview", tag = "items",
    params(("id" = i64, Path, description = "Item id"), PreviewParams),
    responses(
        (status = 200, content_type = "image/jpeg", body = Vec<u8>),
        (status = 404, body = ApiError, description = "No such item, or no preview yet"),
    ))]
async fn item_preview(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(p): Query<PreviewParams>,
    req: Request,
) -> ApiResult<Response> {
    let variant = match (p.page, p.t) {
        (Some(page), _) => format!("p{page}"),
        (None, Some(t)) => format!("t{}", t.max(0.0).floor() as u64),
        (None, None) => "p1".into(),
    };
    let cached = st.svc.cache_path("previews", id, &variant);
    cached_or_image(&st, id, cached, req).await
}

/// Serves a cached rendering if there is one, else the original when it is an image.
async fn cached_or_image(
    st: &Arc<AppState>,
    id: i64,
    cached: std::path::PathBuf,
    req: Request,
) -> ApiResult<Response> {
    if tokio::fs::metadata(&cached).await.is_ok() {
        return serve_file(cached, "image/jpeg", req).await;
    }
    let svc = st.svc.clone();
    let kind = blocking(move || svc.item_kind(id)).await?;
    if kind == Kind::Image {
        return item_raw(State(st.clone()), Path(id), req).await;
    }
    Err(ApiErr(service::Error::NotFound(format!(
        "no rendering of item {id} yet"
    ))))
}

/// The file itself, with HTTP range support for seeking in audio and video.
#[utoipa::path(get, path = "/items/{id}/raw", tag = "items",
    params(("id" = i64, Path, description = "Item id")),
    responses(
        (status = 200, content_type = "application/octet-stream", body = Vec<u8>,
            description = "The file, with its own content type"),
        (status = 206, description = "The requested byte range"),
        (status = 404, body = ApiError),
    ))]
async fn item_raw(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
    req: Request,
) -> ApiResult<Response> {
    let svc = st.svc.clone();
    let (path, mime) = blocking(move || svc.item_path(id)).await?;
    let mime = mime.unwrap_or_else(|| {
        mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string()
    });
    if tokio::fs::metadata(&path).await.is_err() {
        return Err(ApiErr(service::Error::NotFound(format!(
            "the file of item {id} is gone"
        ))));
    }
    let mut resp = serve_file(path.clone().into(), &mime, req).await?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().replace(['"', '\\', '\r', '\n'], "_"))
        .unwrap_or_default();
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{name}\"")) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

async fn serve_file(path: std::path::PathBuf, mime: &str, req: Request) -> ApiResult<Response> {
    let mime =
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let resp = tower_http::services::ServeFile::new_with_mime(path, &mime_from(&mime))
        .oneshot(req)
        .await
        .map_err(|e| service::Error::Internal(e.into()))?;
    Ok(resp.map(Body::new))
}

fn mime_from(v: &HeaderValue) -> mime_guess::Mime {
    v.to_str()
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(mime_guess::mime::APPLICATION_OCTET_STREAM)
}

/// Items that look or sound like this one.
#[utoipa::path(get, path = "/items/{id}/similar", tag = "items",
    params(("id" = i64, Path, description = "Item id"), SimilarParams),
    responses(
        (status = 200, body = SearchResponse),
        (status = 404, body = ApiError),
        (status = 409, body = ApiError, description = "The item is not indexed yet"),
    ))]
async fn item_similar(
    State(st): State<Arc<AppState>>,
    Extension(Prefix(prefix)): Extension<Prefix>,
    Path(id): Path<i64>,
    Query(p): Query<SimilarParams>,
) -> ApiResult<Json<SearchResponse>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.similar(id, &p, &prefix)).await?))
}

/// The settings: the config file's keys, with environment overrides applied.
#[utoipa::path(get, path = "/settings", tag = "settings",
    responses((status = 200, body = SettingsView)))]
async fn get_settings(State(st): State<Arc<AppState>>) -> Json<SettingsView> {
    Json(st.svc.settings())
}

/// Changes settings with a JSON merge patch: send only the keys to change (`null` resets a
/// key to its default). Saved to the config file.
#[utoipa::path(patch, path = "/settings", tag = "settings",
    request_body(content = Config, description = "Any subset of the settings keys"),
    responses((status = 200, body = SettingsView), (status = 400, body = ApiError)))]
async fn patch_settings(
    State(st): State<Arc<AppState>>,
    Json(patch): Json<serde_json::Value>,
) -> ApiResult<Json<SettingsView>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.patch_settings(&patch)).await?))
}

// ---- OpenAPI, docs and UI ----

async fn openapi_json(
    State(st): State<Arc<AppState>>,
    Extension(Prefix(prefix)): Extension<Prefix>,
) -> Json<OpenApiDoc> {
    let mut doc = st.openapi.clone();
    doc.servers = Some(vec![Server::new(if prefix.is_empty() {
        "/".to_string()
    } else {
        prefix
    })]);
    Json(doc)
}

async fn docs() -> Html<&'static str> {
    Html(include_str!("docs.html"))
}

#[derive(rust_embed::Embed)]
#[folder = "ui/"]
struct Ui;

/// Serves the embedded UI; unknown paths get `index.html` so client-side routes work.
async fn ui(
    Extension(Prefix(prefix)): Extension<Prefix>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return ApiErr(service::Error::NotFound(format!("no route {}", uri.path())))
            .into_response();
    }
    if let Some(file) = Ui::get(path).filter(|_| !path.is_empty() && path != "index.html") {
        let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()[..8]));
        if headers
            .get(header::IF_NONE_MATCH)
            .is_some_and(|v| v.as_bytes() == etag.as_bytes())
        {
            return StatusCode::NOT_MODIFIED.into_response();
        }
        return (
            [
                (
                    header::CONTENT_TYPE,
                    mime_guess::from_path(path)
                        .first_or_octet_stream()
                        .to_string(),
                ),
                (header::ETAG, etag),
            ],
            file.data,
        )
            .into_response();
    }
    let Some(index) = Ui::get("index.html") else {
        return (
            StatusCode::NOT_FOUND,
            "the web UI is not built into this binary",
        )
            .into_response();
    };
    let html = String::from_utf8_lossy(&index.data).replacen(
        "<head>",
        &format!("<head>\n<base href=\"{prefix}/\">"),
        1,
    );
    ([(header::CACHE_CONTROL, "no-cache")], Html(html)).into_response()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs the server until Ctrl-C or SIGTERM.
pub async fn serve(svc: Arc<Service>, bind: &str) -> anyhow::Result<()> {
    let cfg: Config = svc.config();
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| anyhow::anyhow!("can't listen on {bind}: {e}"))?;
    let addr = listener.local_addr()?;
    tracing::info!(
        "rummage {} listening on http://{addr}{}/ (embedder: {})",
        env!("CARGO_PKG_VERSION"),
        cfg.base_path(),
        svc.embedder().name()
    );
    let app = app(svc);
    axum::serve(
        listener,
        axum::ServiceExt::<Request>::into_make_service(app),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}
