use std::{
    env::var,
    error::Error,
    ffi::{OsStr, OsString},
    fmt::Debug,
    ops::Deref,
    sync::{Arc, OnceLock},
    time::Duration,
};

use headless_chrome::{Browser, LaunchOptionsBuilder, Tab};
use minijinja::Environment;
use serde::Serialize;

use crate::{
    common::{
        constant::{SHARE_DRIVE_PATH_BUF, TZ},
        domain::ServiceError,
        util::IdGenerator,
    },
    upload::{domain::FileRouterState, service::FileService},
};

use super::{
    constant::{CHROMIUM_SANDBOXED, TEMPL_DEFAULT_DATE_FORMAT, TEMPL_DEFAULT_DATETIME_FORMAT},
    domain::{TemplateType, TemplateV2},
};

static JINJA_ENGINE: OnceLock<Environment<'static>> = OnceLock::new();
pub struct ChromiumTab(OnceLock<(Browser, Arc<Tab>)>);
static CHROMIUM_TAB: ChromiumTab = ChromiumTab(OnceLock::new());

pub async fn init() -> Result<(), Box<dyn Error>> {
    tracing::info!("init jinja...");
    get_jinja_engine();
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

pub async fn render<T: Serialize + Debug>(
    templ: &TemplateV2,
    templ_ctx: &T,
    file_router_state: &FileRouterState,
    tenant: Option<String>,
) -> Result<Vec<u8>, ServiceError> {
    match FileService::get_file_upload(
        &templ.file_id,
        tenant,
        &file_router_state.client,
        &file_router_state.collection,
    )
    .await
    {
        Some((repo, file)) => {
            let file_service = FileService { store: repo };
            let templ_bytes = file_service.download_bytes(&file).await?;

            let result = match templ.template_type {
                TemplateType::Html => html_to_pdf(&templ_bytes, templ_ctx).await,
                TemplateType::Xml => xml_to_xml(&templ_bytes, templ_ctx).await,
            }
            .map_err(|e| {
                ServiceError::new(format!("cannot convert html to pdf: {e} {templ_ctx:?}"))
            })?;

            Ok(result)
        }
        None => Err(ServiceError::new(format!(
            "templ with id {} doesn't seem to exist in db",
            templ.file_id
        ))),
    }
}

async fn xml_to_xml<T: Serialize>(templ: &[u8], templ_ctx: &T) -> Result<Vec<u8>, Box<dyn Error>> {
    let engine = get_jinja_engine();

    tracing::debug!("{}", String::from_utf8_lossy(templ));
    let xml = engine.render_str(std::str::from_utf8(templ)?, templ_ctx)?;
    Ok(xml.into_bytes())
}
async fn html_to_pdf<T: Serialize>(templ: &[u8], templ_ctx: &T) -> Result<Vec<u8>, Box<dyn Error>> {
    let engine = get_jinja_engine();

    tracing::debug!("{}", String::from_utf8_lossy(templ));
    let html = engine.render_str(std::str::from_utf8(templ)?, templ_ctx)?;
    let tab = get_chromium_tab()?;

    let temp_html_file_path = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{}.html", IdGenerator.get()));
    tracing::debug!("{temp_html_file_path:?}");
    tokio::fs::File::create(&temp_html_file_path).await?;
    tokio::fs::write(&temp_html_file_path, html).await?;
    let page = format!("file://{}", temp_html_file_path.display());
    tracing::debug!("{page}");

    tracing::info!("generate pdf from html page {page}");
    let pdf = tab
        .navigate_to(&page)?
        .wait_until_navigated()?
        .print_to_pdf(Default::default())?;
    tokio::fs::remove_file(temp_html_file_path).await?;

    Ok(pdf)
}

fn get_jinja_engine<'a>() -> &'a Environment<'static> {
    JINJA_ENGINE.get_or_init(|| {
        let mut env = Environment::new();
        env.add_global(
            "TIMEZONE",
            var(TZ).unwrap_or_else(|_| "Europe/Brussels".to_string()),
        );
        env.add_global(
            "DATETIME_FORMAT",
            var(TEMPL_DEFAULT_DATETIME_FORMAT)
                .unwrap_or_else(|_| "[day]/[month]/[year] [hour]:[minute]".to_string()),
        );
        env.add_global(
            "DATE_FORMAT",
            var(TEMPL_DEFAULT_DATE_FORMAT).unwrap_or_else(|_| "[day]/[month]/[year]".to_string()),
        );
        env.add_function("round", round);
        minijinja_contrib::add_to_environment(&mut env);
        env
    })
}
fn round(value: f64) -> String {
    let precision = 2;
    let formatted = format!("{:.precision$}", value, precision = precision);
    formatted
}
pub fn get_chromium_tab() -> Result<Arc<Tab>, Box<dyn Error>> {
    match CHROMIUM_TAB.get() {
        Some((_, tab)) => Ok(tab.clone()),
        None => {
            let user_data_dir = OsString::from(format!(
                "--user-data-dir={}",
                SHARE_DRIVE_PATH_BUF.display()
            ));
            let sandboxed = std::env::var(CHROMIUM_SANDBOXED)
                .map(|v| v.parse::<bool>().unwrap_or(false))
                .unwrap_or(false);
            let options = LaunchOptionsBuilder::default()
                .sandbox(sandboxed)
                .idle_browser_timeout(Duration::MAX)
                .args(vec![
                    OsStr::new("--disable-web-security"),
                    OsStr::new("--no-zygote"),
                    OsStr::new("--no-first-run"),
                    //  OsStr::new("--disable-setuid-sandbox"),
                    OsStr::new("--disable-features=IsolateOrigins,site-per-process"),
                    //  OsStr::new("--default-background-color=00000000"),
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
                .set((browser, tab.clone()))
                .map_err(|_tab| "could not setup chromium tab".to_string())?;
            Ok(tab)
        }
    }
}

impl Deref for ChromiumTab {
    type Target = OnceLock<(headless_chrome::Browser, Arc<headless_chrome::Tab>)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for ChromiumTab {
    fn drop(&mut self) {
        if let Some((browser, tab)) = self.0.take() {
            tracing::info!("closing tab result: {:?}", tab.close(true));
            drop(tab);
            drop(browser);
        }
    }
}
#[cfg(test)]
mod test {
    use std::error::Error;

    use chrono::{DateTime, NaiveDate, NaiveDateTime};

    use serde::{Deserialize, Serialize};

    use crate::{common::util::IdGenerator, template::render::get_jinja_engine};

    use super::html_to_pdf;

    #[tokio::test]
    async fn test_html_to_pdf() -> Result<(), Box<dyn Error>> {
        let templ = r#"
        <p>Greeting, {{name}}! You are {{age}} years old!</p>
        <ul>
           {% for stock in stuff.stocks %}
            <li>{{stock}}</li>
           {% endfor %}

        </ul>
        "#;
        let res = html_to_pdf(
            templ.as_bytes(),
            &serde_json::json!({
            "name": "Nordine",
            "age": 35,
            "stuff": {
                "stocks": ["apple", "bananas", "tomatos"]

            }
            }),
        )
        .await?;
        let p = dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("{}.pdf", IdGenerator.get()));
        tokio::fs::write(&p, res).await?;
        println!("path {p:?}");
        Ok(())
    }
    #[tokio::test]
    async fn test_date_and_time() -> Result<(), Box<dyn Error>> {
        #[derive(Serialize, Deserialize)]
        struct Whatever {
            dt: NaiveDateTime,
            d: NaiveDate,
        }

        let ctx = Whatever {
            dt: DateTime::from_timestamp_millis(1662921288000)
                .ok_or("no dt")?
                .naive_utc(),
            d: NaiveDate::from_ymd_opt(2024, 1, 1).ok_or("no nd")?,
        };

        let engine = get_jinja_engine();
        assert_eq!(
            "01/01/2024",
            engine.render_str(r#"{{ d|dateformat }}"#, &ctx)?
        );
        assert_eq!(
            "11/09/2022 18:34",
            engine.render_str(r#"{{ dt|datetimeformat }}"#, &ctx)?
        );

        assert_eq!(
            "11/09/2022 18:34:48",
            engine
                .render_str(r#"{{ dt|datetimeformat(format="[day]/[month]/[year] [hour]:[minute]:[second]",tz='Europe/Paris') }}"#, ctx)
                ?
        );
        Ok(())
    }
}
