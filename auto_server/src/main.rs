use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use bytes::Bytes;
use mime_guess::from_path;
use percent_encoding::percent_decode_str;
use std::path::PathBuf;
use tokio::fs;

const BASE_URL_2025: &str = "https://chunithm.sega.jp/irodorimidori/irodorimidori-rpg-2025/";
const BASE_URL_2024: &str = "https://chunithm.sega.jp/irodorimidori/irodorimidori-rpg/";
const PORT: u16 = 8080;

const IGNORE_PATTERNS: &[&str] = &[".map", ".well-known", "favicon.ico"];

#[get("/{tail:.*}")]
async fn handler(
    req: HttpRequest,
    client: web::Data<reqwest::Client>,
    serve_root: web::Data<PathBuf>,
    base_url: web::Data<String>,
) -> HttpResponse {
    let peer = req
        .peer_addr()
        .map_or_else(|| "unknown".into(), |a| a.ip().to_string());
    println!(
        "\x1b[90m[DEBUG] GET {} from {}\x1b[0m",
        req.uri().path(),
        peer
    );

    let raw_uri_path = req.uri().path().trim_start_matches('/');
    let encoded_path = raw_uri_path.to_owned();
    let clean_path = percent_decode_str(raw_uri_path)
        .decode_utf8_lossy()
        .into_owned();

    let local_name = if clean_path.is_empty() {
        "index.html".to_string()
    } else {
        clean_path.clone()
    };

    let local_path = serve_root.join(&local_name);

    let is_ignored = IGNORE_PATTERNS.iter().any(|p| local_name.contains(p));
    if is_ignored && !local_path.exists() {
        return HttpResponse::NotFound().finish();
    }

    let needs_fetch = !local_path.exists()
        || fs::metadata(&local_path)
            .await
            .map(|m| m.len() == 0)
            .unwrap_or(false);

    if needs_fetch {
        let remote_url = format!("{}{}", base_url.as_str(), encoded_path);
        eprintln!(
            "\x1b[93mDEBUG: Local file missing: {}. Fetching {}…\x1b[0m",
            local_name, remote_url
        );

        if let Some(parent) = local_path.parent() {
            if let Err(e) = fs::create_dir_all(parent).await {
                eprintln!(
                    "FAILED: Could not create directory {}: {}",
                    parent.display(),
                    e
                );
                return HttpResponse::InternalServerError().finish();
            }
        }

        let result = client
            .get(&remote_url)
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                 AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/120.0.0.0 Safari/537.36",
            )
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes().await {
                    Ok(body) => {
                        if let Err(e) = fs::write(&local_path, &body).await {
                            eprintln!("FAILED: Could not write {}: {}", local_name, e);
                            return HttpResponse::InternalServerError().finish();
                        }
                        eprintln!(
                            "\x1b[93mSUCCESS: Downloaded and cached: {}\x1b[0m",
                            local_name
                        );
                        // Serve the just-downloaded bytes directly
                        return serve_bytes(body, &local_name);
                    }
                    Err(e) => {
                        eprintln!("FAILED: Could not read remote body {}: {}", remote_url, e);
                        return HttpResponse::NotFound()
                            .body("File not found on remote server either.");
                    }
                }
            }
            Ok(resp) => {
                eprintln!(
                    "FAILED: Remote returned {} for {}",
                    resp.status(),
                    remote_url
                );
                return HttpResponse::NotFound().body("File not found on remote server either.");
            }
            Err(e) => {
                eprintln!("FAILED: Could not fetch {}: {}", remote_url, e);
                return HttpResponse::NotFound().body("File not found on remote server either.");
            }
        }
    }

    match fs::read(&local_path).await {
        Ok(data) => serve_bytes(Bytes::from(data), &local_name),
        Err(e) => {
            eprintln!("ERROR reading {}: {}", local_path.display(), e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

fn serve_bytes(data: Bytes, filename: &str) -> HttpResponse {
    let content_type = from_path(filename).first_or_octet_stream().to_string();
    HttpResponse::Ok().content_type(content_type).body(data)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let raw_root: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(exe_dir);

    let rpg_version: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2025);

    let base_url = if rpg_version == 2024 {
        BASE_URL_2024
    } else {
        BASE_URL_2025
    };

    let absolute_root = if raw_root.is_absolute() {
        raw_root
    } else {
        std::env::current_dir()?.join(raw_root)
    };

    if !absolute_root.exists() {
        std::fs::create_dir_all(&absolute_root)?;
    }

    std::env::set_current_dir(&absolute_root)?;

    println!("Serving  : {}", absolute_root.display());
    println!("Upstream : {}", base_url);
    println!("Listening: http://0.0.0.0:{}", PORT);
    println!("Press Ctrl+C to stop.\n");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to create HTTP client");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(client.clone()))
            .app_data(web::Data::new(absolute_root.clone()))
            .app_data(web::Data::new(base_url.to_string()))
            .service(handler)
    })
    .bind(("0.0.0.0", PORT))?
    .run()
    .await
}
