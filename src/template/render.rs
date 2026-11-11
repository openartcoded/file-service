use std::{
    env::var,
    error::Error,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, Once},
    time::Duration,
};

use chrono::Local;
use headless_chrome::{
    Browser, LaunchOptionsBuilder, Tab, protocol::cdp::Emulation::SetEmulatedMedia,
    types::PrintToPdfOptions,
};
use mime_guess::mime::{APPLICATION_PDF, TEXT_XML};
use mongodb::bson::doc;
use serde::Serialize;
use tokio::time::{Instant, sleep_until};

use crate::{
    common::{constant::TMP_FS_PATH, domain::ServiceError, util::IdGenerator},
    store::{Repository, StoreRepository},
    upload::{
        domain::{FileRouterState, FileUploadV2},
        service::FileService,
    },
};

use super::{
    constant::{
        A4_HEIGHT_MM, A4_WIDTH_MM, CHROMIUM_SANDBOXED, PDF_MARGIN_MM, PDF_MEDIA, PDF_SCALE,
    },
    domain::{TemplRouterState, TemplateType, TemplateV2},
    engine::{self, SourceLoader},
};

/// The chromium instance used to print pdfs. One tab shared by every request, so the mutex also
/// serializes the renderings (otherwise a navigation can replace the page another request is
/// printing). None until the first launch, and again after a reset: the next rendering relaunches.
static CHROMIUM: Mutex<Option<Chromium>> = Mutex::new(None);
static CLEARING_SCHEDULED: Once = Once::new();

struct Chromium {
    browser: Browser,
    tab: Arc<Tab>,
    data_dir: PathBuf,
}

impl Drop for Chromium {
    fn drop(&mut self) {
        tracing::info!("closing tab result: {:?}", self.tab.close(true));
        tracing::info!(
            "chromium process {:?} closed",
            self.browser.get_process_id()
        );
        tracing::info!(
            "deleting user data folder (chromium) {:?}: {:?}",
            self.data_dir,
            std::fs::remove_dir_all(&self.data_dir)
        );
    }
}

pub async fn init() -> Result<(), Box<dyn Error>> {
    tracing::info!("init jinja...");
    engine::new_engine(None);
    tracing::info!("init jinja done!");
    tracing::info!("init chromium...");
    schedule_clearing();
    tokio::task::spawn_blocking(move || {
        loop {
            match with_chromium(|_| Ok(())) {
                Ok(_) => {
                    tracing::info!("init chrome done!");
                    break;
                }
                Err(e) => tracing::warn!("chrome not available yet! will try again in a sec...{e}"),
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    })
    .await?;
    Ok(())
}

/// A rendered template
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub content_type: String,
    /// with the dot, e.g. ".pdf"
    pub extension: String,
}

/// The source of a template, and its file (name, content type).
pub async fn load_source(
    templ: &TemplateV2,
    file_router_state: &FileRouterState,
    tenant: &str,
) -> Result<(String, FileUploadV2), ServiceError> {
    match FileService::get_file_upload(
        &templ.file_id,
        Some(tenant.to_string()),
        &file_router_state.client,
        &file_router_state.collection,
    )
    .await
    {
        Some((repo, file)) => {
            let file_service = FileService { store: repo };
            let bytes = file_service.download_bytes(&file).await?;
            let source = String::from_utf8(bytes).map_err(|e| {
                ServiceError::new(format!("template {} is not utf-8: {e}", templ.id))
            })?;
            Ok((source, file))
        }
        None => Err(ServiceError::new(format!(
            "templ with id {} doesn't seem to exist in db",
            templ.file_id
        ))),
    }
}

/// `{% include "x" %}` / `{% extends "x" %}` / `{% import "x" %}`: x is the id or the title of another template
fn include_loader(
    templ_router_state: TemplRouterState,
    file_router_state: FileRouterState,
    tenant: String,
) -> SourceLoader {
    let handle = tokio::runtime::Handle::current();
    Arc::new(move |name: &str| {
        // the engine is synchronous, the store is not
        tokio::task::block_in_place(|| {
            handle.block_on(find_source(
                &templ_router_state,
                &file_router_state,
                &tenant,
                name,
            ))
        })
        .map_err(|e| {
            minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                format!("cannot load template {name}: {e}"),
            )
        })
    })
}

async fn find_source(
    templ_router_state: &TemplRouterState,
    file_router_state: &FileRouterState,
    tenant: &str,
    name: &str,
) -> Result<Option<String>, ServiceError> {
    let repository: StoreRepository<TemplateV2> = StoreRepository::get_repository(
        &templ_router_state.client,
        &templ_router_state.collection.0,
        tenant,
    );
    let templ = match repository
        .find_by_id(name)
        .await
        .map_err(ServiceError::new)?
    {
        Some(templ) => Some(templ),
        None => repository
            .find_one(Some(doc! {"title": name}))
            .await
            .map_err(ServiceError::new)?,
    };
    match templ {
        Some(templ) => load_source(&templ, file_router_state, tenant)
            .await
            .map(|(source, _)| Some(source)),
        None => Ok(None),
    }
}

pub async fn render<T: Serialize + Debug>(
    templ: &TemplateV2,
    templ_ctx: &T,
    file_router_state: &FileRouterState,
    templ_router_state: &TemplRouterState,
    tenant: &str,
) -> Result<Rendered, ServiceError> {
    let (source, file) = load_source(templ, file_router_state, tenant).await?;
    let env = engine::new_engine(Some(include_loader(
        templ_router_state.clone(),
        file_router_state.clone(),
        tenant.to_string(),
    )));
    // the context is not logged: it can be large (an embedded pdf) and personal
    let output = engine::render_str(&env, &source, templ_ctx).map_err(|e| {
        ServiceError::new(format!(
            "cannot render template {} ({}): {e:#}",
            templ.title, templ.id
        ))
    })?;

    match templ.template_type {
        TemplateType::Html => {
            let bytes = html_to_pdf(output)
                .await
                .map_err(|e| ServiceError::new(format!("cannot convert html to pdf: {e}")))?;
            Ok(Rendered {
                bytes,
                content_type: APPLICATION_PDF.to_string(),
                extension: ".pdf".to_string(),
            })
        }
        TemplateType::Xml => Ok(Rendered {
            bytes: output.into_bytes(),
            content_type: TEXT_XML.to_string(),
            extension: ".xml".to_string(),
        }),
        TemplateType::Text => {
            let (content_type, extension) = text_type(&file.original_filename);
            Ok(Rendered {
                bytes: output.into_bytes(),
                content_type,
                extension,
            })
        }
    }
}

/// content type and extension of a text template, from its file name: email.html, invite.ics, sms.txt...
fn text_type(file_name: &str) -> (String, String) {
    let mime = mime_guess::from_path(file_name).first_or_text_plain();
    let content_type = if mime.type_() == mime_guess::mime::TEXT {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    let extension = Path::new(file_name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_else(|| ".txt".to_string());
    (content_type, extension)
}

async fn html_to_pdf(html: String) -> Result<Vec<u8>, Box<dyn Error>> {
    let temp_html_file_path = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{}.html", IdGenerator.get()));
    tracing::debug!("{temp_html_file_path:?}");
    tokio::fs::write(&temp_html_file_path, html).await?;
    let page = format!("file://{}", temp_html_file_path.display());
    tracing::debug!("{page}");

    tracing::info!("generate pdf from html page {page}");
    let pdf = tokio::task::block_in_place(|| {
        with_chromium(|tab| {
            if let Some(media) = pdf_media() {
                tab.call_method(SetEmulatedMedia {
                    media: Some(media),
                    features: None,
                })?;
            }
            Ok(tab
                .navigate_to(&page)?
                .wait_until_navigated()?
                .print_to_pdf(Some(pdf_options()))?)
        })
        // not kept as Box<dyn Error> across the await below (not Send)
        .map_err(ServiceError::new)
    });
    // removed whatever the outcome
    if let Err(e) = tokio::fs::remove_file(&temp_html_file_path).await {
        tracing::warn!("could not remove {temp_html_file_path:?}: {e}");
    }

    Ok(pdf?)
}

/// A4, 10mm margins, backgrounds printed. Same defaults as the backend (PdfToolBox), so a
/// template looks the same whichever renders it. An @page rule (size, margin) in the template wins.
fn pdf_options() -> PrintToPdfOptions {
    const MM_PER_INCH: f64 = 25.4;
    let margin = PDF_MARGIN_MM / MM_PER_INCH;
    PrintToPdfOptions {
        paper_width: Some(A4_WIDTH_MM / MM_PER_INCH),
        paper_height: Some(A4_HEIGHT_MM / MM_PER_INCH),
        margin_top: Some(margin),
        margin_bottom: Some(margin),
        margin_left: Some(margin),
        margin_right: Some(margin),
        print_background: Some(true),
        prefer_css_page_size: Some(true),
        scale: Some(pdf_scale()),
        ..Default::default()
    }
}

/// PDF_SCALE (0.1 - 2, default 1)
fn pdf_scale() -> f64 {
    var(PDF_SCALE)
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|s| (0.1..=2.0).contains(s))
        .unwrap_or(1.0)
}

/// PDF_MEDIA: css media type used to print (default "screen", like the backend; empty: chromium's
/// default, "print")
fn pdf_media() -> Option<String> {
    let media = var(PDF_MEDIA).unwrap_or_else(|_| "screen".to_string());
    let media = media.trim();
    (!media.is_empty()).then(|| media.to_string())
}

/// Runs `f` with the shared chromium tab, launching chromium first if needed. When the call fails
/// because chromium (or the tab) is gone, e.g. a crashed renderer: "No session with given id", the
/// instance is thrown away and `f` is retried once on a freshly launched one. Before, the dead tab
/// was kept forever and every rendering failed until the service was restarted.
fn with_chromium<T>(f: impl Fn(&Tab) -> Result<T, Box<dyn Error>>) -> Result<T, Box<dyn Error>> {
    let mut chromium = lock_chromium();
    for attempt in 1..=2 {
        if chromium.is_none() {
            *chromium = Some(launch_chromium()?);
        }
        let Some(current) = chromium.as_ref() else {
            unreachable!("chromium launched above")
        };
        match f(current.tab.as_ref()) {
            Ok(value) => return Ok(value),
            Err(e) if attempt == 1 && is_broken(current, e.as_ref()) => {
                tracing::warn!("chromium is broken ({e}), relaunching it and trying again");
                // drop: closes chromium and deletes its user data dir
                *chromium = None;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("the second attempt always returns")
}

fn lock_chromium() -> MutexGuard<'static, Option<Chromium>> {
    CHROMIUM
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The error comes from a dead tab / browser rather than from the page being rendered
fn is_broken(chromium: &Chromium, e: &dyn Error) -> bool {
    let msg = e.to_string().to_lowercase();
    [
        "no session",
        "no target",
        "closed",
        "connection",
        "disconnected",
        "channel",
    ]
    .iter()
    .any(|m| msg.contains(m))
        || chromium.browser.get_version().is_err()
}

/// CLEARING_CHROMIUM_USER_DATA_DIR: every night, chromium is closed and its user data dir deleted,
/// the next rendering relaunches it with a fresh one. Before, the dir was deleted under the running
/// chromium, which could leave it (and every following rendering) broken.
fn schedule_clearing() {
    let enabled = var("CLEARING_CHROMIUM_USER_DATA_DIR")
        .ok()
        .and_then(|p| p.trim().to_lowercase().parse::<bool>().ok())
        .unwrap_or(false);
    if !enabled {
        return;
    }
    CLEARING_SCHEDULED.call_once(|| {
        tokio::spawn(async move {
            loop {
                let now = Local::now();
                let tomorrow_midnight = (now + chrono::Duration::days(1))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .and_then(|d| d.and_local_timezone(Local).earliest())
                    .unwrap_or(now + chrono::Duration::days(1));
                let duration_until_midnight = (tomorrow_midnight - now)
                    .to_std()
                    .unwrap_or(Duration::from_secs(24 * 3600));
                tracing::info!(
                    "chromium --user-data-dir (tmpfs): next clearing schedule is {tomorrow_midnight}"
                );
                sleep_until(Instant::now() + duration_until_midnight).await;
                let cleared = tokio::task::spawn_blocking(|| {
                    // waits for the rendering in progress, if any
                    lock_chromium().take().is_some()
                })
                .await;
                tracing::info!("chromium closed and its user data dir cleared: {cleared:?}");
            }
        });
    });
}

fn launch_chromium() -> Result<Chromium, Box<dyn Error>> {
    let data_dir = TMP_FS_PATH.join(IdGenerator.get());
    std::fs::create_dir_all(&data_dir)?;

    let user_data_dir = OsString::from(format!("--user-data-dir={}", data_dir.display()));
    let sandboxed = std::env::var(CHROMIUM_SANDBOXED)
        .map(|v| v.parse::<bool>().unwrap_or(false))
        .unwrap_or(false);
    let options = LaunchOptionsBuilder::default()
        .sandbox(sandboxed)
        //.fetcher_options(FetcherOptions::default().with_rRevision, evision(Revision::Latest))
        .idle_browser_timeout(Duration::MAX)
        .args(vec![
            OsStr::new("--disable-web-security"),
            OsStr::new("--no-zygote"),
            OsStr::new("--no-first-run"),
            //  OsStr::new("--disable-setuid-sandbox"),
            OsStr::new("--allow-file-access-from-files"),
            OsStr::new("--enable-features=NetworkService,NetworkServiceInProcess"),
            OsStr::new("--enable-local-file-accesses"),
            OsStr::new("--disable-pdf-tagging"),
            OsStr::new("--disable-features=IsolateOrigins,site-per-process"),
            OsStr::new("--default-background-color=00000000"),
            OsStr::new("--disable-dev-shm-usage"),
            &user_data_dir,
        ])
        .build()
        .map_err(|e| format!("invalid options: {e}"))?;

    tracing::info!("chromium opts: {options:?}");

    let browser = Browser::new(options)?;
    std::thread::sleep(Duration::from_secs(2));
    tracing::info!("we got a browser: {:?}", browser.get_process_id());
    for attempt in 1..=10 {
        match browser.get_version() {
            Ok(version) => {
                tracing::info!("browser ready: {}", version.product);
                break;
            }
            Err(e) if attempt < 10 => {
                tracing::warn!("waiting for browser (attempt {}): {}", attempt, e);
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                return Err(format!("browser not ready after 10 attempts: {}", e).into());
            }
        }
    }

    let tab = match browser.new_tab() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("failed to create tab: {}", e);
            return Err(e.into());
        }
    };

    // Wait for tab to be ready
    std::thread::sleep(Duration::from_millis(200));

    tab.activate()?;
    tracing::info!("tab activated successfully");

    tracing::info!("we got a tab");

    Ok(Chromium {
        browser,
        tab,
        data_dir,
    })
}

#[cfg(test)]
mod test {
    use std::error::Error;

    use serde_json::json;

    use crate::{common::util::IdGenerator, template::engine};

    use super::{html_to_pdf, text_type};

    #[test]
    fn text_templates_get_the_content_type_of_their_file_name() {
        assert_eq!(
            ("text/html; charset=utf-8".to_string(), ".html".to_string()),
            text_type("email.html")
        );
        assert_eq!(
            (
                "text/calendar; charset=utf-8".to_string(),
                ".ics".to_string()
            ),
            text_type("invite.ics")
        );
        assert_eq!(
            ("text/plain; charset=utf-8".to_string(), ".txt".to_string()),
            text_type("sms")
        );
    }

    // block_in_place needs the multi-threaded runtime; run with `cargo test -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a chromium"]
    async fn test_html_to_pdf() -> Result<(), Box<dyn Error>> {
        let templ = r#"
        <p>Greeting, {{name}}! You are {{age}} years old!</p>
        <ul>
           {% for stock in stuff.stocks %}
            <li>{{stock}}</li>
           {% endfor %}
        </ul>
        "#;
        let html = engine::render_str(
            &engine::new_engine(None),
            templ,
            &json!({
                "name": "Nordine",
                "age": 35,
                "stuff": {"stocks": ["apple", "bananas", "tomatos"]}
            }),
        )?;
        let res = html_to_pdf(html).await?;
        assert!(res.starts_with(b"%PDF"));
        let p = dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("{}.pdf", IdGenerator.get()));
        tokio::fs::write(&p, res).await?;
        println!("path {p:?}");
        Ok(())
    }
}
