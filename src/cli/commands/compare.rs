//! `compare`: two periods side by side, joined on the dimension keys
//! (decision 11).

use crate::api::SEARCH_ANALYTICS_QUERY;
use crate::cli::commands::query::{
    SearchArgs, check_sort, print_header, print_rows, resolve, warn_before_retention,
    warn_truncated,
};
use crate::cli::commands::{Context, Leaf};
use crate::cli::spec::CommandSpec;
use crate::dates;
use crate::error::Error;
use crate::output::{self, Format, warn};
use crate::search::{
    Cap, Day, Order, PAGE_MAX, Range, Spec, compare_columns, compare_rows, compare_totals, fetch,
    sort,
};
use clap::Args;
use serde_json::{Map, Value, json};
use time::Date;

pub(crate) static SPEC: CommandSpec = CommandSpec {
    summary: "Compare two periods: each row's metrics, previous metrics and change",
    endpoints: &[SEARCH_ANALYTICS_QUERY],
    token_endpoint: false,
    tabular: true,
    list_key: Some("rows"),
    dry_run: true,
    example: "gsc-cli compare --site sc-domain:example.com --dimensions page --days 7 --sort clicksChange --order asc",
};

/// Rows printed when neither --limit nor --all is given.
const DEFAULT_LIMIT: u64 = 100;

/// Two periods side by side: for each row, the metrics now, before, and the
/// change (a negative positionChange is an improvement). By default the
/// previous period is the one of the same length just before. A row seen in
/// one period only has 0 clicks and impressions, and no CTR or position, in
/// the other. The totals are asked for separately, so they include the
/// anonymized queries the rows leave out.
#[derive(Args, Debug)]
pub(crate) struct Compare {
    #[command(flatten)]
    search: SearchArgs,

    /// Compare with the same weekdays 52 weeks earlier.
    #[arg(long, conflicts_with = "previous_start_date")]
    yoy: bool,

    /// Start the previous period on this day (YYYY-MM-DD); it has the same
    /// length as the current one.
    #[arg(long, value_parser = dates::parse, value_name = "DATE")]
    previous_start_date: Option<Date>,

    /// Sort by a column: a dimension, or clicks, impressions, ctr, position,
    /// each also with Previous or Change (clicksChange).
    #[arg(long, value_name = "COLUMN", default_value = "clicks")]
    sort: String,

    /// The --sort direction.
    #[arg(long, value_enum, default_value_t = Order::Desc)]
    order: Order,

    /// Print at most N rows. Default 100.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), value_name = "N", conflicts_with = "all")]
    limit: Option<u64>,

    /// Print every row.
    #[arg(long)]
    all: bool,

    /// The most rows to fetch for each period. Both periods are fetched in
    /// full before the join, so that a row that fell out of the top shows.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), default_value_t = 100_000, value_name = "N")]
    max_rows: u64,
}

impl Compare {
    fn previous(&self, current: &Range) -> Range {
        match (self.previous_start_date, self.yoy) {
            (Some(start), _) => current.starting(start),
            (None, true) => current.year_before(),
            (None, false) => current.previous(),
        }
    }
}

impl Leaf for Compare {
    fn spec(&self) -> &'static CommandSpec {
        &SPEC
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let spec = self.search.spec();
        let range = self.search.range()?;
        let columns = compare_columns(&spec.dimensions);
        check_sort(&self.sort, &columns)?;
        let site = self.search.site();
        let grouped = !spec.dimensions.is_empty();

        if ctx.dry_run {
            return self.dry_run(ctx, &spec, &range);
        }

        let (start, end) = resolve(ctx, site, &spec, &range)?;
        let current = Range {
            start: Day::Fixed(start),
            end: Day::Fixed(end),
        };
        let (previous_start, previous_end) = self.previous(&current).resolve(end)?;
        warn_before_retention(previous_start, dates::today());
        let previous = Range {
            start: Day::Fixed(previous_start),
            end: Day::Fixed(previous_end),
        };

        let api = ctx.api.clone();
        let mut send = |body: Value| ctx.send(&api.search_analytics(site, body)?);
        let totals_now = send(Value::Object(spec.body(current.start, current.end, false)))?;
        let totals_before = send(Value::Object(spec.body(
            previous.start,
            previous.end,
            false,
        )))?;
        let totals = compare_totals(&totals_now, &totals_before);

        let mut rows = if grouped {
            let (now, now_capped) = all_rows(&spec, &current, self.max_rows, &mut send)?;
            let (before, before_capped) = all_rows(&spec, &previous, self.max_rows, &mut send)?;
            if now_capped || before_capped {
                warn(
                    "row_cap",
                    &format!(
                        "fetched the first {} rows of a period only (--max-rows), so rows past \
                         them compare against 0",
                        self.max_rows
                    ),
                );
            }
            compare_rows(&now, &before, &spec.dimensions)
        } else {
            vec![totals.clone()]
        };
        sort(&mut rows, &self.sort, self.order);
        let keep = if self.all {
            rows.len()
        } else {
            usize::try_from(self.limit.unwrap_or(DEFAULT_LIMIT)).unwrap_or(usize::MAX)
        };
        let truncated = rows.len() > keep;
        rows.truncate(keep);
        if truncated {
            warn_truncated(rows.len());
        }

        let mut request = spec.body(current.start, current.end, true);
        request.shift_remove("startDate");
        request.shift_remove("endDate");
        let mut envelope = Map::new();
        envelope.insert("siteUrl".into(), site.as_str().into());
        envelope.insert("current".into(), period(start, end));
        envelope.insert("previous".into(), period(previous_start, previous_end));
        envelope.insert("request".into(), Value::Object(request));
        envelope.insert("totals".into(), totals);
        print(envelope, rows, truncated, &columns, ctx.format);
        Ok(())
    }
}

impl Compare {
    /// The probe (when needed), both totals, and both first pages of rows.
    fn dry_run(&self, ctx: &Context, spec: &Spec, range: &Range) -> Result<(), Error> {
        let site = self.search.site();
        let previous = self.previous(range);
        let mut requests = Vec::new();
        if range.needs_latest() {
            requests.push(ctx.api.search_analytics(site, spec.probe(dates::today()))?);
        }
        for period in [range, &previous] {
            let body = spec.body(period.start, period.end, false);
            requests.push(ctx.api.search_analytics(site, Value::Object(body))?);
        }
        if !spec.dimensions.is_empty() {
            for period in [range, &previous] {
                let mut body = spec.body(period.start, period.end, true);
                body.insert("rowLimit".into(), (self.max_rows + 1).min(PAGE_MAX).into());
                body.insert("startRow".into(), 0.into());
                requests.push(ctx.api.search_analytics(site, Value::Object(body))?);
            }
        }
        ctx.print_dry_run(&requests);
        Ok(())
    }
}

fn period(start: Date, end: Date) -> Value {
    json!({"startDate": dates::format(start), "endDate": dates::format(end)})
}

/// json: the envelope with the rows; jsonl and csv: the rows only.
fn print(
    mut envelope: Map<String, Value>,
    rows: Vec<Value>,
    truncated: bool,
    columns: &[String],
    format: Format,
) {
    match format {
        Format::Json => {
            envelope.insert("rowCount".into(), rows.len().into());
            envelope.insert("truncated".into(), truncated.into());
            envelope.insert("rows".into(), Value::Array(rows));
            output::print(&Value::Object(envelope), Format::Json);
        }
        Format::Jsonl => {
            print_rows(&rows, columns, Format::Jsonl);
        }
        Format::Csv => {
            if print_header(columns) {
                print_rows(&rows, columns, Format::Csv);
            }
        }
    }
}

/// Every row of one period, up to `max_rows`, as Google sent them; and
/// whether the cap cut them.
fn all_rows(
    spec: &Spec,
    period: &Range,
    max_rows: u64,
    send: &mut impl FnMut(Value) -> Result<Value, Error>,
) -> Result<(Vec<Value>, bool), Error> {
    let mut rows = Vec::new();
    let body = spec.body(period.start, period.end, true);
    let paged = fetch(&body, Cap::Rows(max_rows), send, |page| {
        rows.extend_from_slice(page);
        true
    })?;
    Ok((rows, paged.truncated))
}
