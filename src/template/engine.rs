//! The jinja (minijinja 3) environment templates are rendered with.
//!
//! Besides the builtins, templates get:
//! - `{% include %}`, `{% import %}`, `{% extends %}` of other templates (by id or title), see [SourceLoader]
//! - the contrib date filters (`dateformat`, `datetimeformat`, `timeformat`, `now()`), with strftime formats
//! - python methods (`"x".upper()`, `d.items()`, `"{} {}".format(a, b)`...), `tojson`, `break` / `continue`
//! - `currency`, `number` and `nl2br` filters, and the legacy `round()` function
//! - a fuel limit, so a runaway template fails instead of blocking the service
use std::{env::var, sync::Arc};

use minijinja::{
    Environment, Error, ErrorKind,
    value::{Kwargs, Serde, Value},
};
use serde::Serialize;

use super::constant::{TEMPL_DEFAULT_DATE_FORMAT, TEMPL_DEFAULT_DATETIME_FORMAT, TEMPL_FUEL};
use crate::common::constant::TZ;

/// strftime format, overridable with TEMPL_DEFAULT_DATETIME_FORMAT
pub const DEFAULT_DATETIME_FORMAT: &str = "%d/%m/%Y %H:%M";
/// strftime format, overridable with TEMPL_DEFAULT_DATE_FORMAT
pub const DEFAULT_DATE_FORMAT: &str = "%d/%m/%Y";
/// instructions a rendering can execute (TEMPL_FUEL, 0 disables the limit). Far above what a real document needs.
pub const DEFAULT_FUEL: u64 = 50_000_000;

/// Gives the source of the template named in an include, import or extends. `Ok(None)`: no such template.
pub type SourceLoader = Arc<dyn Fn(&str) -> Result<Option<String>, Error> + Send + Sync>;

/// A new environment. One per rendering: included templates are loaded fresh, never from a stale cache.
pub fn new_engine(loader: Option<SourceLoader>) -> Environment<'static> {
    build_engine(loader, fuel())
}

fn build_engine(loader: Option<SourceLoader>, fuel: Option<u64>) -> Environment<'static> {
    let mut env = Environment::new();
    env.add_global(
        "TIMEZONE",
        var(TZ).unwrap_or_else(|_| "Europe/Brussels".to_string()),
    );
    env.add_global(
        "DATETIME_FORMAT",
        var(TEMPL_DEFAULT_DATETIME_FORMAT).unwrap_or_else(|_| DEFAULT_DATETIME_FORMAT.to_string()),
    );
    env.add_global(
        "DATE_FORMAT",
        var(TEMPL_DEFAULT_DATE_FORMAT).unwrap_or_else(|_| DEFAULT_DATE_FORMAT.to_string()),
    );
    env.add_function("round", round);
    env.add_filter("currency", currency);
    env.add_filter("number", number);
    env.add_filter("nl2br", nl2br);
    minijinja_contrib::add_to_environment(&mut env);
    env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    env.set_fuel(fuel);
    if let Some(loader) = loader {
        env.set_loader(move |name: &str| loader(name));
    }
    env
}

fn fuel() -> Option<u64> {
    match var(TEMPL_FUEL)
        .ok()
        .and_then(|f| f.trim().parse::<u64>().ok())
    {
        Some(0) => None,
        Some(fuel) => Some(fuel),
        None => Some(DEFAULT_FUEL),
    }
}

/// Renders a template source with any serializable context.
pub fn render_str<T: Serialize + ?Sized>(
    env: &Environment<'static>,
    source: &str,
    ctx: &T,
) -> Result<String, Error> {
    env.render_str(source, Serde(ctx))
}

/// Compiles a template: a syntax error is reported with its line. Includes are not resolved.
pub fn validate(source: &str) -> Result<(), Error> {
    let env = new_engine(None);
    env.template_from_str(source).map(|_| ())
}

/// The variables a template reads without setting them (e.g. `invoice.total`), sorted.
pub fn variables(source: &str) -> Result<Vec<String>, Error> {
    let env = new_engine(None);
    let template = env.template_from_str(source)?;
    let mut variables: Vec<String> = template.undeclared_variables(true).into_iter().collect();
    variables.sort();
    Ok(variables)
}

/// legacy: `round(1.005)` gives "1.00" (two decimals, as a string)
fn round(value: f64) -> String {
    format!("{value:.2}")
}

/// `{{ 1234.5|number }}` gives `1.234,50`. kwargs: decimals (2), thousands ("."), decimal (",")
fn number(value: f64, kwargs: Kwargs) -> Result<String, Error> {
    let format = NumberFormat::from_kwargs(&kwargs)?;
    kwargs.assert_all_used()?;
    Ok(format.format(value))
}

/// `{{ 1234.5|currency }}` gives `1.234,50 €`. kwargs: those of `number`, plus symbol ("€") and
/// symbol_first (false: `€ 1.234,50` when true)
fn currency(value: f64, kwargs: Kwargs) -> Result<String, Error> {
    let format = NumberFormat::from_kwargs(&kwargs)?;
    let symbol = kwargs
        .get::<Option<String>>("symbol")?
        .unwrap_or_else(|| "€".to_string());
    let symbol_first = kwargs.get::<Option<bool>>("symbol_first")?.unwrap_or(false);
    kwargs.assert_all_used()?;
    let amount = format.format(value);
    Ok(if symbol_first {
        format!("{symbol} {amount}")
    } else {
        format!("{amount} {symbol}")
    })
}

/// line breaks to `<br>` (e.g. a description typed in a textarea)
fn nl2br(value: String) -> Value {
    Value::from_safe_string(value.replace("\r\n", "\n").replace('\n', "<br>"))
}

struct NumberFormat {
    decimals: usize,
    thousands: String,
    decimal: String,
}

impl NumberFormat {
    fn from_kwargs(kwargs: &Kwargs) -> Result<Self, Error> {
        let decimals = kwargs.get::<Option<i64>>("decimals")?.unwrap_or(2);
        if !(0..=10).contains(&decimals) {
            return Err(Error::new(
                ErrorKind::InvalidOperation,
                "decimals must be between 0 and 10",
            ));
        }
        Ok(NumberFormat {
            decimals: decimals as usize,
            thousands: kwargs
                .get::<Option<String>>("thousands")?
                .unwrap_or_else(|| ".".to_string()),
            decimal: kwargs
                .get::<Option<String>>("decimal")?
                .unwrap_or_else(|| ",".to_string()),
        })
    }

    fn format(&self, value: f64) -> String {
        let rounded = format!("{:.*}", self.decimals, value.abs());
        let (int_part, frac_part) = match rounded.split_once('.') {
            Some((int_part, frac_part)) => (int_part, Some(frac_part)),
            None => (rounded.as_str(), None),
        };
        let mut out = String::with_capacity(rounded.len() + 8);
        // no "-0,00"
        if value < 0.0 && rounded.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
            out.push('-');
        }
        let len = int_part.len();
        for (i, c) in int_part.chars().enumerate() {
            if i > 0 && (len - i) % 3 == 0 {
                out.push_str(&self.thousands);
            }
            out.push(c);
        }
        if let Some(frac_part) = frac_part {
            out.push_str(&self.decimal);
            out.push_str(frac_part);
        }
        out
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error as StdError, sync::Arc};

    use chrono::{DateTime, NaiveDate, Utc};
    use minijinja::ErrorKind;
    use serde::Serialize;
    use serde_json::json;

    use super::{SourceLoader, build_engine, new_engine, render_str, validate, variables};

    fn render(source: &str, ctx: serde_json::Value) -> Result<String, minijinja::Error> {
        render_str(&new_engine(None), source, &ctx)
    }

    #[test]
    fn dates_use_strftime_formats_and_time_zones() -> Result<(), Box<dyn StdError>> {
        #[derive(Serialize)]
        struct Ctx {
            utc: DateTime<Utc>,
            d: NaiveDate,
        }
        let ctx = Ctx {
            utc: DateTime::from_timestamp_millis(1662921288000).ok_or("no dt")?,
            d: NaiveDate::from_ymd_opt(2024, 1, 1).ok_or("no date")?,
        };
        let env = new_engine(None);

        assert_eq!("01/01/2024", render_str(&env, "{{ d|dateformat }}", &ctx)?);
        // 18:34:48 UTC is 20:34:48 in Brussels (summer time)
        assert_eq!(
            "11/09/2022 20:34",
            render_str(
                &env,
                r#"{{ utc|datetimeformat(tz="Europe/Brussels") }}"#,
                &ctx
            )?
        );
        assert_eq!(
            "11/09/2022 18:34:48",
            render_str(
                &env,
                r#"{{ utc|datetimeformat(format="%d/%m/%Y %H:%M:%S", tz="UTC") }}"#,
                &ctx
            )?
        );
        Ok(())
    }

    #[test]
    fn numbers_and_currencies() -> Result<(), Box<dyn StdError>> {
        let ctx = json!({"total": 1234.5, "big": -1234567.891, "tiny": -0.001, "n": 12});
        assert_eq!("1.234,50 €", render("{{ total|currency }}", ctx.clone())?);
        assert_eq!(
            "-1 234 567.9",
            render(
                r#"{{ big|number(decimals=1, thousands=" ", decimal=".") }}"#,
                ctx.clone()
            )?
        );
        assert_eq!("0,00", render("{{ tiny|number }}", ctx.clone())?);
        assert_eq!(
            "$ 12.00",
            render(
                r#"{{ n|currency(symbol="$", symbol_first=true, thousands=",", decimal=".") }}"#,
                ctx.clone()
            )?
        );
        assert_eq!("1234.50", render("{{ round(total) }}", ctx.clone())?);
        assert!(render("{{ total|number(oops=1) }}", ctx).is_err());
        Ok(())
    }

    #[test]
    fn nl2br_python_methods_loop_controls_and_json() -> Result<(), Box<dyn StdError>> {
        #[derive(Serialize)]
        struct Ordered {
            b: i32,
            a: i32,
        }
        #[derive(Serialize)]
        struct Ctx {
            text: &'static str,
            name: &'static str,
            m: Ordered,
        }
        let ctx = Ctx {
            text: "a\nb\r\nc",
            name: "acme",
            m: Ordered { b: 1, a: 2 },
        };
        let env = new_engine(None);
        assert_eq!("a<br>b<br>c", render_str(&env, "{{ text|nl2br }}", &ctx)?);
        assert_eq!("ACME", render_str(&env, "{{ name.upper() }}", &ctx)?);
        // keys keep their order
        assert_eq!(
            "b=1;a=2;",
            render_str(
                &env,
                "{% for k, v in m.items() %}{{ k }}={{ v }};{% endfor %}",
                &ctx
            )?
        );
        assert_eq!(
            r#"{"b": 1, "a": 2}"#,
            render_str(&env, "{{ m|tojson }}", &ctx)?
        );
        assert_eq!(
            "1",
            render_str(
                &env,
                "{% for i in [1, 2, 3] %}{% if i == 2 %}{% break %}{% endif %}{{ i }}{% endfor %}",
                &ctx
            )?
        );
        Ok(())
    }

    #[test]
    fn other_templates_can_be_included_and_extended() -> Result<(), Box<dyn StdError>> {
        let templates = HashMap::from([
            ("footer".to_string(), "-- {{ name }}".to_string()),
            (
                "layout".to_string(),
                "<{% block body %}{% endblock %}>".to_string(),
            ),
            (
                "macros".to_string(),
                "{% macro hi(who) %}hi {{ who }}{% endmacro %}".to_string(),
            ),
        ]);
        let loader: SourceLoader = Arc::new(move |name: &str| Ok(templates.get(name).cloned()));
        let env = new_engine(Some(loader));
        let ctx = json!({"name": "ACME"});

        assert_eq!(
            "Hi -- ACME",
            render_str(&env, r#"Hi {% include "footer" %}"#, &ctx)?
        );
        assert_eq!(
            "<x>",
            render_str(
                &env,
                r#"{% extends "layout" %}{% block body %}x{% endblock %}"#,
                &ctx
            )?
        );
        assert_eq!(
            "hi ACME",
            render_str(&env, r#"{% from "macros" import hi %}{{ hi(name) }}"#, &ctx)?
        );
        assert!(render_str(&env, r#"{% include "missing" %}"#, &ctx).is_err());
        Ok(())
    }

    #[test]
    fn a_runaway_template_runs_out_of_fuel() {
        let env = build_engine(None, Some(1_000));
        let err = render_str(
            &env,
            "{% for i in range(1000) %}{% for j in range(1000) %}{{ j }}{% endfor %}{% endfor %}",
            &json!({}),
        )
        .expect_err("should run out of fuel");
        assert_eq!(ErrorKind::OutOfFuel, err.kind());
    }

    #[test]
    fn templates_are_validated_and_their_variables_listed() -> Result<(), Box<dyn StdError>> {
        assert!(validate("{% for x in items %}{{ x }}").is_err());
        assert!(validate("{{ invoice.total|currency }}").is_ok());
        assert_eq!(
            vec!["invoice.total".to_string(), "name".to_string()],
            variables("{{ invoice.total }}{% set a = 1 %}{{ a }} {{ name }}")?
        );
        Ok(())
    }
}
