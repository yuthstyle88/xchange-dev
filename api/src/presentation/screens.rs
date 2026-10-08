#[path = "../../build_support/screens.rs"]
mod generator;

pub use generator::{Gate, ScreenDecl, ScreenTableError};

use crate::bootstrap::AppState;
use crate::presentation::pages;
use crate::presentation::ui::UiAssets;
use actix_web::{HttpRequest, HttpResponse, web};
use std::path::{Path, PathBuf};
use xc_auth::domain::policy::value_objects::role::Role;

#[derive(Debug)]
pub enum ScreenLoadError {
    Io(String),
    Table(ScreenTableError),
}

#[derive(Clone)]
enum AssetSource {
    Embedded,
    Dir(PathBuf),
}

include!(concat!(env!("OUT_DIR"), "/v3_screens.rs"));

#[derive(Clone)]
pub struct ScreenTable {
    entries: Vec<ScreenDecl>,
    source: AssetSource,
    built: bool,
}

impl ScreenTable {
    pub fn compiled() -> Self {
        let entries = SCREENS
            .iter()
            .map(|&(path, gate, entry)| ScreenDecl {
                path: path.to_string(),
                gate,
                entry: entry.to_string(),
            })
            .collect();
        let built = UiAssets::built();
        Self {
            entries,
            source: AssetSource::Embedded,
            built,
        }
    }

    pub fn from_dist(dir: &Path) -> Result<Self, ScreenLoadError> {
        let entries_raw =
            generator::read_entries(dir).map_err(|e| ScreenLoadError::Io(e.to_string()))?;
        let entry_refs: Vec<(&str, &str)> = entries_raw
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let entries = generator::screen_table(&entry_refs).map_err(ScreenLoadError::Table)?;
        let built = dir.join(".vite/manifest.json").is_file();
        Ok(Self {
            entries,
            source: AssetSource::Dir(dir.to_path_buf()),
            built,
        })
    }

    pub fn entries(&self) -> &[ScreenDecl] {
        &self.entries
    }

    pub fn holds(&self, path: &str) -> bool {
        self.entries.iter().any(|d| d.path == path)
    }

    pub fn built(&self) -> bool {
        self.built
    }

    fn read_entry_bytes(&self, entry: &str) -> Option<Vec<u8>> {
        match &self.source {
            AssetSource::Embedded => UiAssets::get(entry).map(|f| f.data.into_owned()),
            AssetSource::Dir(dir) => std::fs::read(dir.join(entry)).ok(),
        }
    }

    fn read_asset(&self, tail: &str) -> Option<(Vec<u8>, &'static str)> {
        let mime = if tail.ends_with(".js") || tail.ends_with(".mjs") {
            "text/javascript; charset=utf-8"
        } else if tail.ends_with(".css") {
            "text/css; charset=utf-8"
        } else if tail.ends_with(".json") {
            "application/json"
        } else if tail.ends_with(".html") {
            "text/html; charset=utf-8"
        } else if tail.ends_with(".svg") {
            "image/svg+xml"
        } else if tail.ends_with(".png") {
            "image/png"
        } else if tail.ends_with(".woff2") {
            "font/woff2"
        } else {
            "application/octet-stream"
        };

        let bytes = match &self.source {
            AssetSource::Embedded => UiAssets::get(tail).map(|f| f.data.into_owned())?,
            AssetSource::Dir(dir) => {
                let path = dir.join(tail);
                if !path.is_file() {
                    return None;
                }
                std::fs::read(path).ok()?
            }
        };
        Some((bytes, mime))
    }

    pub(crate) async fn serve(&self, req: &HttpRequest) -> HttpResponse {
        if req.method() != actix_web::http::Method::GET {
            return pages::plain_404();
        }

        let Some(decl) = self.entries.iter().find(|d| d.path == req.path()) else {
            return pages::plain_404();
        };

        match decl.gate {
            Gate::Anyone => self.render_screen(decl),
            Gate::SignedIn => {
                let sessions = pages::sessions_of(req).await;
                if sessions.owner.is_none() && sessions.staff.is_none() {
                    pages::to_login(req)
                } else {
                    self.render_screen(decl)
                }
            }
            Gate::Branch => {
                let sessions = pages::sessions_of(req).await;
                if sessions.owner.is_none() && sessions.staff.is_none() {
                    pages::to_login(req)
                } else {
                    match pages::page_branch(req, &sessions) {
                        Ok(_branch) => self.render_screen(decl),
                        Err(resp) => resp,
                    }
                }
            }
            Gate::Front => {
                let sessions = pages::sessions_of(req).await;
                let is_front = matches!(&sessions.staff, Some(s) if s.role() == Role::Front);
                if !is_front {
                    pages::to_staff(req)
                } else {
                    self.render_screen(decl)
                }
            }
        }
    }

    fn render_screen(&self, decl: &ScreenDecl) -> HttpResponse {
        let Some(bytes) = self.read_entry_bytes(&decl.entry) else {
            return pages::plain_404();
        };

        HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .insert_header(("Cache-Control", "no-cache"))
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .body(bytes)
    }

    pub(crate) async fn serve_asset(&self, _req: &HttpRequest, tail: &str) -> HttpResponse {
        if !self.built {
            return HttpResponse::ServiceUnavailable()
                .content_type("text/plain; charset=utf-8")
                .body("ui not built");
        }

        if !tail.starts_with("assets/")
            || tail.contains("..")
            || tail.contains("%2e")
            || tail.contains("%2E")
        {
            return pages::plain_404();
        }

        let Some((bytes, mime)) = self.read_asset(tail) else {
            return pages::plain_404();
        };

        HttpResponse::Ok()
            .content_type(mime)
            .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .body(bytes)
    }

    pub fn configure(self, cfg: &mut web::ServiceConfig, _state: &AppState) {
        let table = web::Data::new(self.clone());
        cfg.app_data(table.clone());

        for decl in self.entries() {
            let path = decl.path.clone();
            let tbl = self.clone();
            cfg.service(
                web::resource(&path)
                    .route(web::get().to(move |req: HttpRequest| {
                        let tbl = tbl.clone();
                        async move { tbl.serve(&req).await }
                    }))
                    .default_service(web::to(pages::fallback)),
            );
        }

        let tbl = self.clone();
        cfg.service(
            web::resource("/ui")
                .route(web::get().to(move |req: HttpRequest| {
                    let tbl = tbl.clone();
                    async move { tbl.serve_asset(&req, "").await }
                }))
                .default_service(web::to(pages::fallback)),
        );
        let tbl = self.clone();
        cfg.service(
            web::resource("/ui/{tail:.*}")
                .route(
                    web::get().to(move |req: HttpRequest, path: web::Path<String>| {
                        let tbl = tbl.clone();
                        let tail = path.into_inner();
                        async move { tbl.serve_asset(&req, &tail).await }
                    }),
                )
                .default_service(web::to(pages::fallback)),
        );
    }
}
