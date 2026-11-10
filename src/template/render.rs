use std::{
    env::var,
    error::Error,
    ffi::{OsStr, OsString},
    fmt::Debug,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
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

pub struct ChromiumTab(OnceLock<(Browser, Arc<Tab>, PathBuf)>);
static CHROMIUM_TAB: ChromiumTab = ChromiumTab(OnceLock::new());
static RENDER_LOCK: Mutex<()> = Mutex::new(());

pub async fn init() -> Result<(), Box<dyn Error>> {
    tracing::info!("init jinja...");
    engine::new_engine(None);
    tracing::info!("init jinja done!");
    tracing::info!("init chromium...");
    tokio::task::spawn_blocking(move || {
        loop {
            match get_chromium_tab() {
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
        // one chromium tab shared by every request: one rendering at a time, otherwise
        // a navigation can replace the page another request is printing
        let _guard = RENDER_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tab = get_chromium_tab().map_err(ServiceError::new)?;
        if let Some(media) = pdf_media() {
            tab.call_method(SetEmulatedMedia {
                media: Some(media),
                features: None,
            })
            .map_err(ServiceError::new)?;
        }

        tab.navigate_to(&page)
            .map_err(ServiceError::new)?
            .wait_until_navigated()
            .map_err(ServiceError::new)?
            .print_to_pdf(Some(pdf_options()))
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

pub fn get_chromium_tab() -> Result<Arc<Tab>, Box<dyn Error>> {
    match CHROMIUM_TAB.get() {
        Some((_, tab, _)) => Ok(tab.clone()),
        None => {
            let data_dir = TMP_FS_PATH.join(IdGenerator.get());
            std::fs::create_dir_all(&data_dir)?;

            if let Some(true) = std::env::var("CLEARING_CHROMIUM_USER_DATA_DIR")
                .map(|p| p.trim().to_lowercase().parse::<bool>().ok())
                .iter()
                .flatten()
                .next()
            {
                let cleanup_dir = data_dir.clone();
                let handle = tokio::runtime::Handle::current();
                handle.spawn(async move {
                            loop {
                                tracing::info!("starting next clearing procedure of the chrome user data dir");
                                let now = Local::now();
                                let tomorrow_midnight = (now + chrono::Duration::days(1))
                                    .date_naive()
                                    .and_hms_opt(0, 0, 0)
                                    .unwrap()
                                    .and_local_timezone(Local)
                                    .unwrap();

                                let duration_until_midnight = (tomorrow_midnight - now).to_std().unwrap();

                                let sleep_time = Instant::now() + duration_until_midnight;

                                tracing::info!(
                                    "chromium --user-data-dir (tmpfs): next clearing schedule is {tomorrow_midnight}, data_dir: {cleanup_dir:?}",
                                );
                                sleep_until(sleep_time).await;
                                tracing::info!(
                                    "clearing the chromium user data dir, this might be messy! {:?}",
                                    tokio::fs::remove_dir_all(&cleanup_dir).await
                                );
                                tracing::info!(
                                    "recreate dir {:?}",
                                    tokio::fs::create_dir_all(&cleanup_dir).await
                                );
                            }
                        });
            }
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

            CHROMIUM_TAB
                .set((browser, tab.clone(), data_dir))
                .map_err(|_tab| "could not setup chromium tab".to_string())?;
            Ok(tab)
        }
    }
}

impl Deref for ChromiumTab {
    type Target = OnceLock<(headless_chrome::Browser, Arc<headless_chrome::Tab>, PathBuf)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for ChromiumTab {
    fn drop(&mut self) {
        if let Some((browser, tab, pb)) = self.0.take() {
            tracing::info!("closing tab result: {:?}", tab.close(true));
            drop(tab);
            drop(browser);
            tracing::info!(
                "deleting user data folder (chromium) {:?}",
                std::fs::remove_dir_all(pb)
            );
        }
    }
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
