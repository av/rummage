//! API round trips over the real router, with the deterministic fake embedder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use rummage::client::{Api, Client};
use rummage::server;
use rummage_core::service::Service;
use rummage_core::store::{ChunkRow, NewFile};
use rummage_core::types::*;
use rummage_core::{ConfigFile, EmbedInput, Embedder, FakeEmbedder, ImageData};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    dir: tempfile::TempDir,
    svc: Arc<Service>,
    ids: Vec<i64>,
}

impl Fixture {
    fn new(extra_config: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(
            &config,
            format!(
                "data_dir = \"{}\"\n{extra_config}",
                dir.path().join("data").display()
            ),
        )
        .unwrap();
        let cf = ConfigFile::load_with_env(&config, |_| None).unwrap();
        let embedder = Arc::new(FakeEmbedder::new(768));
        let svc = Arc::new(Service::open(cf, embedder.clone()).unwrap());
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        let mut ids = vec![];
        for (name, kind, text) in [
            (
                "bike.md",
                Kind::Text,
                "a red bicycle leaning on the garden fence",
            ),
            ("tax.md", Kind::Text, "quarterly tax invoice for 2024"),
            ("photo.jpg", Kind::Image, ""),
        ] {
            let path = docs.join(name);
            let bytes = if kind == Kind::Image {
                b"\xff\xd8\xff\xe0 not really a jpeg".to_vec()
            } else {
                text.as_bytes().to_vec()
            };
            std::fs::write(&path, &bytes).unwrap();
            ids.push(seed(&svc, &embedder, &path, &docs, kind, text, bytes));
        }
        Fixture { dir, svc, ids }
    }

    fn app(&self) -> server::App {
        server::app(self.svc.clone())
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("config.toml")).unwrap()
    }
}

/// Stores a file as the indexer would: a file row, one chunk, one vector.
fn seed(
    svc: &Service,
    embedder: &FakeEmbedder,
    path: &Path,
    folder: &Path,
    kind: Kind,
    text: &str,
    bytes: Vec<u8>,
) -> i64 {
    let store = svc.store();
    let id = store
        .upsert_file(&NewFile {
            path: path.to_string_lossy().into(),
            folder: folder.to_string_lossy().into(),
            size: bytes.len() as i64,
            mtime: 1_700_000_000,
            kind,
            mime: Some(
                mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
            ),
            ..Default::default()
        })
        .unwrap();
    let (seg, input) = if kind == Kind::Image {
        (
            SegmentKind::Image,
            EmbedInput::Image(ImageData::Encoded(bytes)),
        )
    } else {
        (
            SegmentKind::Text,
            EmbedInput::Document {
                title: None,
                text: text.into(),
            },
        )
    };
    let chunk = store
        .insert_chunk(&ChunkRow {
            file_id: id,
            kind: seg,
            text: (!text.is_empty()).then(|| text.to_string()),
            ..Default::default()
        })
        .unwrap();
    let v = embedder.embed(&[input]).unwrap().pop().unwrap();
    store.put_vector(chunk, &v).unwrap();
    store.set_status(id, "indexed", None).unwrap();
    id
}

async fn call<S>(app: &S, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>)
where
    S: tower::Service<
            Request<Body>,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone,
{
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn json_req(method: Method, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(body)))
}

#[tokio::test]
async fn health_and_openapi() {
    let f = Fixture::new("");
    let app = f.app();
    let (status, _, body) = call(&app, get("/api/v1/health")).await;
    assert_eq!(status, StatusCode::OK);
    let h: Health = parse(&body);
    assert_eq!((h.status.as_str(), h.embedder.dim), ("ok", 768));
    assert!(!h.auth_required);

    let (status, _, body) = call(&app, get("/api/openapi.json")).await;
    assert_eq!(status, StatusCode::OK);
    let spec: Value = parse(&body);
    assert_eq!(spec["servers"][0]["url"], "/");
    let paths: Vec<&str> = spec["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for p in [
        "/api/v1/health",
        "/api/v1/folders",
        "/api/v1/folders/{id}",
        "/api/v1/index/status",
        "/api/v1/index/events",
        "/api/v1/reindex",
        "/api/v1/search",
        "/api/v1/items/{id}",
        "/api/v1/items/{id}/thumb",
        "/api/v1/items/{id}/preview",
        "/api/v1/items/{id}/raw",
        "/api/v1/items/{id}/similar",
        "/api/v1/settings",
    ] {
        assert!(paths.contains(&p), "{p} missing from {paths:?}");
    }
    assert!(spec["components"]["schemas"]["SearchRequest"].is_object());

    let (status, _, body) = call(&app, get("/api/docs")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("openapi.json"));

    let (status, _, body) = call(&app, get("/api/v1/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(parse::<ApiError>(&body).error, "not_found");
}

#[tokio::test]
async fn search_round_trip() {
    let f = Fixture::new("");
    let app = f.app();
    let (status, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/search",
            json!({ "query": "red bicycle" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let r: SearchResponse = parse(&body);
    let top = &r.hits[0];
    assert_eq!(top.item.name, "bike.md");
    assert!(top.keyword_match);
    // Two of the eight words match: cosine about sqrt(2/8) with the fake embedder.
    assert!(top.similarity.unwrap() > 0.4, "{:?}", top.similarity);
    assert!(r.hits[1..].iter().all(|h| h.similarity < top.similarity));
    assert_eq!(
        top.item.urls.raw,
        format!("/api/v1/items/{}/raw", top.item.id)
    );
    assert!(
        top.segment
            .as_ref()
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .contains("bicycle")
    );

    // Filters.
    let (_, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/search",
            json!({ "query": "red bicycle", "filters": { "types": ["image"] } }),
        ),
    )
    .await;
    let r: SearchResponse = parse(&body);
    assert!(r.hits.iter().all(|h| h.item.kind == Kind::Image));
    let (_, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/search",
            json!({ "query": "invoice", "mode": "keyword", "filters": { "path_glob": "**/tax.*" } }),
        ),
    )
    .await;
    let r: SearchResponse = parse(&body);
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].item.name, "tax.md");

    // Bad requests.
    let (status, _, body) = call(&app, json_req(Method::POST, "/api/v1/search", json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse::<ApiError>(&body).error, "bad_request");

    // Item detail, similar.
    let bike = f.ids[0];
    let (status, _, body) = call(&app, get(&format!("/api/v1/items/{bike}?q=fence"))).await;
    assert_eq!(status, StatusCode::OK);
    let d: ItemDetail = parse(&body);
    assert_eq!(d.segments.len(), 1);
    assert!(d.segments[0].score.is_some());
    let (_, _, body) = call(&app, get(&format!("/api/v1/items/{bike}/similar?limit=5"))).await;
    let r: SearchResponse = parse(&body);
    assert!(!r.hits.is_empty());
    assert!(r.hits.iter().all(|h| h.item.id != bike));
    let (status, _, _) = call(&app, get("/api/v1/items/999")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_by_uploaded_file() {
    let f = Fixture::new("");
    let app = f.app();
    let photo = std::fs::read(f.dir.path().join("docs/photo.jpg")).unwrap();
    let boundary = "XyZ";
    let mut body = Vec::new();
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\n\r\n{{\"limit\": 2}}\r\n").as_bytes());
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"q.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").as_bytes());
    body.extend(&photo);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::post("/api/v1/search")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, _, body) = call(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let r: SearchResponse = parse(&body);
    assert_eq!(r.limit, 2);
    assert_eq!(r.hits[0].item.name, "photo.jpg");
    assert!(r.hits[0].similarity.unwrap() > 0.99);

    // The same file inline as base64.
    use base64::Engine as _;
    let (status, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/search",
            json!({ "file": { "name": "q.jpg", "data_base64": base64::engine::general_purpose::STANDARD.encode(&photo) } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse::<SearchResponse>(&body).hits[0].item.name,
        "photo.jpg"
    );
}

#[tokio::test]
async fn raw_supports_ranges_and_thumb_falls_back() {
    let f = Fixture::new("");
    let app = f.app();
    let bike = f.ids[0];
    let req = Request::get(format!("/api/v1/items/{bike}/raw"))
        .header(header::RANGE, "bytes=2-4")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = call(&app, req).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"red");
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/markdown")
    );
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("bike.md")
    );

    let photo = f.ids[2];
    let (status, _, _) = call(&app, get(&format!("/api/v1/items/{photo}/thumb"))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = call(&app, get(&format!("/api/v1/items/{bike}/preview?page=1"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(parse::<ApiError>(&body).error, "not_found");
}

#[tokio::test]
async fn folders_crud_writes_the_config_file() {
    let f = Fixture::new("");
    let app = f.app();
    let pics = f.dir.path().join("pics");
    std::fs::create_dir_all(&pics).unwrap();
    let pics = std::fs::canonicalize(pics).unwrap();

    let (status, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/folders",
            json!({ "path": pics, "exclude": ["**/*.raw"] }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let folder: Folder = parse(&body);
    assert!(folder.available && folder.enabled);
    assert!(f.config_text().contains("**/*.raw"));

    let (status, _, _) = call(
        &app,
        json_req(Method::POST, "/api/v1/folders", json!({ "path": pics })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/folders",
            json!({ "path": "relative/dir" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let uri = format!("/api/v1/folders/{}", folder.id);
    let (status, _, body) = call(
        &app,
        json_req(Method::PATCH, &uri, json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!parse::<Folder>(&body).enabled);
    assert!(f.config_text().contains("enabled = false"));

    let (_, _, body) = call(&app, get("/api/v1/folders")).await;
    assert_eq!(parse::<Vec<Folder>>(&body).len(), 1);

    let (status, _, _) = call(&app, Request::delete(&uri).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(&app, get(&uri)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!f.config_text().contains(pics.to_str().unwrap()));
}

#[tokio::test]
async fn settings_status_and_reindex() {
    let f = Fixture::new("[server]\nauth_token = \"\"\n");
    let app = f.app();
    let (status, _, body) = call(
        &app,
        json_req(
            Method::PATCH,
            "/api/v1/settings",
            json!({ "index": { "thumbnail_size": 400 }, "model": { "dim": 256 } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: SettingsView = parse(&body);
    assert_eq!(view.settings.index.thumbnail_size, 400);
    assert_eq!(view.restart_required, vec!["model.dim"]);
    assert!(f.config_text().contains("thumbnail_size = 400"));

    let (status, _, body) = call(
        &app,
        json_req(
            Method::PATCH,
            "/api/v1/settings",
            json!({ "model": { "dim": 300 } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(parse::<ApiError>(&body).message.contains("model.dim"));

    let (_, _, body) = call(&app, get("/api/v1/index/status")).await;
    let s: IndexStatus = parse(&body);
    assert_eq!((s.totals.files, s.totals.indexed, s.queued), (3, 3, 0));
    assert_eq!(s.by_kind.len(), 2);

    let (status, _, body) = call(
        &app,
        json_req(Method::POST, "/api/v1/reindex", json!({ "scope": "all" })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(parse::<ReindexResponse>(&body).queued, 3);
    let (status, _, _) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/reindex",
            json!({ "scope": "item", "id": 999 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn events_stream_starts_with_status() {
    let f = Fixture::new("");
    let app = f.app();
    let resp = app.oneshot(get("/api/v1/index/events")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut body = resp.into_body();
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let text = String::from_utf8_lossy(&frame);
    assert!(text.starts_with("event: status\ndata: {"), "{text}");
    assert!(text.contains("\"type\":\"status\""));
}

#[tokio::test]
async fn base_path_and_forwarded_prefix() {
    let f = Fixture::new("[server]\nbase_path = \"/search\"\n");
    let app = f.app();
    let (status, _, _) = call(&app, get("/search/api/v1/health")).await;
    assert_eq!(status, StatusCode::OK);
    // A proxy that strips the prefix still works.
    let (status, _, _) = call(&app, get("/api/v1/health")).await;
    assert_eq!(status, StatusCode::OK);

    let (status, headers, _) = call(&app, get("/search?q=x")).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers[header::LOCATION], "/search/?q=x");

    let (status, _, body) = call(&app, get("/search/")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("<base href=\"/search/\">"));
    let (_, _, body) = call(&app, get("/search/some/client/route")).await;
    assert!(String::from_utf8_lossy(&body).contains("<base href=\"/search/\">"));

    let (_, _, body) = call(&app, get("/search/api/openapi.json")).await;
    assert_eq!(parse::<Value>(&body)["servers"][0]["url"], "/search");

    let (_, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/search/api/v1/search",
            json!({ "query": "tax" }),
        ),
    )
    .await;
    assert!(
        parse::<SearchResponse>(&body).hits[0]
            .item
            .urls
            .thumb
            .starts_with("/search/api/v1/items/")
    );

    let mut req = json_req(Method::POST, "/api/v1/search", json!({ "query": "tax" }));
    req.headers_mut()
        .insert("x-forwarded-prefix", "/proxied/".parse().unwrap());
    let (_, _, body) = call(&app, req).await;
    assert!(
        parse::<SearchResponse>(&body).hits[0]
            .item
            .urls
            .thumb
            .starts_with("/proxied/api/v1/items/")
    );
}

#[tokio::test]
async fn auth_token_guards_the_api() {
    let f = Fixture::new("[server]\nauth_token = \"s3cret\"\n");
    let app = f.app();
    let (status, _, body) = call(&app, get("/api/v1/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(parse::<Health>(&body).auth_required);

    let (status, headers, body) = call(&app, get("/api/v1/folders")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    assert_eq!(parse::<ApiError>(&body).error, "unauthorized");

    let req = Request::get("/api/v1/folders")
        .header(header::AUTHORIZATION, "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&app, req).await.0, StatusCode::OK);
    let id = f.ids[0];
    assert_eq!(
        call(
            &app,
            get(&format!("/api/v1/items/{id}/raw?access_token=s3cret"))
        )
        .await
        .0,
        StatusCode::OK
    );
    let req = Request::get("/api/v1/folders")
        .header(header::AUTHORIZATION, "Bearer wrong!")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&app, req).await.0, StatusCode::UNAUTHORIZED);

    let (_, _, body) = call(
        &app,
        Request::get("/api/v1/settings")
            .header(header::AUTHORIZATION, "Bearer s3cret")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        parse::<SettingsView>(&body).settings.server.auth_token,
        "********"
    );
}

/// The CLI's HTTP client against a real listening server.
#[tokio::test(flavor = "multi_thread")]
async fn cli_client_talks_to_a_live_server() {
    let f = Fixture::new("[server]\nbase_path = \"/search\"\nauth_token = \"t\"\n");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = f.app();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::ServiceExt::<axum::extract::Request>::into_make_service(app),
        )
        .await
        .unwrap();
    });
    let base = format!("http://{addr}/search");
    let config: PathBuf = f.dir.path().join("config.toml");
    let hits = tokio::task::spawn_blocking(move || {
        let api = Api::Remote(Client::new(&base, "t"));
        assert!(api.health().unwrap().auth_required);
        assert_eq!(api.status().unwrap().totals.files, 3);
        let r = api
            .search(&SearchRequest {
                query: Some("quarterly invoice".into()),
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        // What `rummage item` and `rummage folders set` send.
        let d = api.item(r.hits[0].item.id, Some("invoice")).unwrap();
        assert_eq!(d.item.name, "tax.md");
        assert!(d.segments[0].score.is_some());
        let docs = config.parent().unwrap().join("docs");
        let added = api
            .add_folder(NewFolder {
                path: docs.to_string_lossy().into_owned(),
                include: vec![],
                exclude: vec![],
                enabled: true,
            })
            .unwrap();
        let patch = FolderPatch {
            enabled: Some(false),
            exclude: Some(vec!["**/*.tmp".into()]),
            include: None,
        };
        let f = api.update_folder(&added.id, &patch).unwrap();
        assert!(!f.enabled);
        assert_eq!(f.exclude, ["**/*.tmp"]);
        // Discovery finds the same server from the config file.
        let cf = ConfigFile::load_with_env(&config, |_| None).unwrap();
        let found = Api::connect(cf, Some(&base), false).unwrap();
        assert!(matches!(found, Api::Remote(_)));
        assert!(Api::Remote(Client::new(&base, "wrong")).folders().is_err());
        r.hits
    })
    .await
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.name, "tax.md");
}

#[tokio::test]
async fn kind_counts_dirs_and_waveform() {
    let f = Fixture::new("");
    let app = f.app();
    // Counts per kind ignore the `types` filter, so chips can show every kind.
    let (_, _, body) = call(
        &app,
        json_req(
            Method::POST,
            "/api/v1/search",
            json!({ "query": "red bicycle", "mode": "vector", "filters": { "types": ["image"] } }),
        ),
    )
    .await;
    let r: SearchResponse = parse(&body);
    assert!(r.hits.iter().all(|h| h.item.kind == Kind::Image));
    let count = |k: Kind| r.kinds.iter().find(|c| c.kind == k).map_or(0, |c| c.files);
    assert_eq!((count(Kind::Text), count(Kind::Image)), (2, 1));

    let docs = f.dir.path().join("docs");
    std::fs::create_dir_all(docs.join("sub")).unwrap();
    std::fs::create_dir_all(docs.join(".hidden")).unwrap();
    let uri = format!("/api/v1/fs/dirs?path={}", docs.display());
    let (status, _, body) = call(&app, get(&uri)).await;
    assert_eq!(status, StatusCode::OK);
    let l: DirListing = parse(&body);
    assert_eq!(l.dirs.len(), 1);
    assert_eq!(l.dirs[0].name, "sub");
    assert!(!l.dirs[0].watched);
    let (_, _, body) = call(&app, get(&format!("{uri}&hidden=true"))).await;
    assert_eq!(parse::<DirListing>(&body).dirs.len(), 2);
    let (status, _, _) = call(&app, get("/api/v1/fs/dirs?path=relative")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = call(&app, get(&format!("/api/v1/items/{}/waveform", f.ids[0]))).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}
