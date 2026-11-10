//! `query`: Search Analytics rows, paged, with named keys (decisions 3, 9,
//! 10).

use crate::api::SEARCH_ANALYTICS_QUERY;
use crate::cli::commands::{Context, Leaf, SiteArgs};
use crate::cli::spec::CommandSpec;
use crate::dates;
use crate::error::Error;
use crate::output::{self, Format, csv_line, warn};
use crate::search::{
    AggregationType, Cap, DataState, Day, Dimension, Filter, Order, PAGE_MAX, Range, SearchType,
    Spec, fetch, latest_in, named, query_columns, sort,
};
use crate::site::Site;
use clap::Args;
use serde_json::{Map, Value, json};
use time::Date;

pub(crate) static SPEC: CommandSpec = CommandSpec {
    summary: "Clicks, impressions, CTR and position, grouped by dimensions",
    endpoints: &[SEARCH_ANALYTICS_QUERY],
    token_endpoint: false,
    tabular: true,
    list_key: Some("rows"),
    dry_run: true,
    example: "gsc-cli query --site sc-domain:example.com --dimensions page --filter page:includingRegex:/blog/ --days 28",
};

/// Rows printed when neither --limit nor --all is given.
const DEFAULT_LIMIT: u64 = 1000;

/// The flags `query` and `compare` share: what to ask for, and when.
#[derive(Args, Debug)]
pub(crate) struct SearchArgs {
    #[command(flatten)]
    pub(crate) site: SiteArgs,

    /// Group the rows by these dimensions, comma-separated. Without
    /// dimensions, one row of totals.
    #[arg(
        long,
        short = 'd',
        value_enum,
        value_delimiter = ',',
        ignore_case = true
    )]
    dimensions: Vec<Dimension>,

    /// Keep the rows where the filter holds; repeat it for AND (for OR, use
    /// one regex with |). Dimensions: query, page, country, device,
    /// searchAppearance. Operators: equals, notEquals, contains, notContains,
    /// includingRegex, excludingRegex (RE2 syntax). Sends
    /// dimensionFilterGroups.
    #[arg(
        long = "filter",
        short = 'f',
        value_parser = Filter::parse,
        value_name = "DIMENSION:OPERATOR:EXPRESSION"
    )]
    filters: Vec<Filter>,

    /// Where the results were shown (sends type).
    #[arg(long = "type", value_enum, ignore_case = true, default_value_t = SearchType::Web)]
    search_type: SearchType,

    /// final: only final data, about 2-3 days behind. all: also fresh data
    /// that may still change. `hourly_all`: fresh data by hour, with the hour
    /// dimension.
    #[arg(long, value_enum, ignore_case = true, default_value_t = DataState::Final)]
    data_state: DataState,

    /// How Google adds up rows. By default it is byPage when you group or
    /// filter by page, else byProperty, and the totals differ between the
    /// two.
    #[arg(long, value_enum, ignore_case = true)]
    aggregation_type: Option<AggregationType>,

    /// The first day, YYYY-MM-DD, in Pacific Time.
    #[arg(long, value_parser = dates::parse, value_name = "DATE")]
    start_date: Option<Date>,

    /// The last day, YYYY-MM-DD, in Pacific Time. Default: the latest day
    /// with data, found with one more request.
    #[arg(long, value_parser = dates::parse, value_name = "DATE")]
    end_date: Option<Date>,

    /// Days in the range, counted back from --end-date (or forward from
    /// --start-date). Default 28.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=600), value_name = "N")]
    days: Option<u32>,
}

impl SearchArgs {
    pub(crate) fn site(&self) -> &Site {
        &self.site.site
    }

    pub(crate) fn spec(&self) -> Spec {
        Spec {
            dimensions: self.dimensions.clone(),
            filters: self.filters.clone(),
            search_type: self.search_type,
            data_state: self.data_state,
            aggregation_type: self.aggregation_type,
        }
    }

    pub(crate) fn range(&self) -> Result<Range, Error> {
        Range::from_flags(self.start_date, self.end_date, self.days)
    }
}

/// The dates of `range`, after the probe when it ends on the latest day with
/// data (decision 9).
pub(crate) fn resolve(
    ctx: &mut Context,
    site: &Site,
    spec: &Spec,
    range: &Range,
) -> Result<(Date, Date), Error> {
    let today = dates::today();
    let latest = if range.needs_latest() {
        let probe = ctx.api.search_analytics(site, spec.probe(today))?;
        let answer = ctx.send(&probe)?;
        latest_in(&answer).unwrap_or_else(|| {
            warn(
                "no_recent_data",
                "Search Console has no data for the last 16 days, so the range ends 3 days ago; \
                 pass --end-date to choose",
            );
            dates::minus(today, 3)
        })
    } else {
        today
    };
    let (start, end) = range.resolve(latest)?;
    warn_before_retention(start, today);
    Ok((start, end))
}

/// Google returns no rows, and no error, for days it no longer keeps. It
/// keeps some days more than 16 months, so the rows "may be" missing.
pub(crate) fn warn_before_retention(start: Date, today: Date) {
    let oldest = dates::months_before(today, dates::RETENTION_MONTHS);
    if start < oldest {
        warn(
            "before_retention",
            &format!(
                "Search Console keeps about 16 months of data: rows before about {} may be missing",
                dates::format(oldest)
            ),
        );
    }
}

/// A `config` error unless `field` is one of `columns`.
pub(crate) fn check_sort(field: &str, columns: &[String]) -> Result<(), Error> {
    if columns.iter().any(|column| column == field) {
        Ok(())
    } else {
        Err(Error::config(format!(
            "--sort {field:?} is not a column of this output: use one of {}",
            columns.join(", ")
        )))
    }
}

/// Prints rows as jsonl or csv lines, until stdout closes. Returns `false`
/// then.
pub(crate) fn print_rows(rows: &[Value], columns: &[String], format: Format) -> bool {
    rows.iter().all(|row| match format {
        Format::Csv => {
            let cells: Vec<&Value> = columns
                .iter()
                .map(|column| row.get(column).unwrap_or(&Value::Null))
                .collect();
            crate::out!("{}", csv_line(cells))
        }
        Format::Json | Format::Jsonl => output::print(row, Format::Jsonl),
    })
}

/// The CSV header line. Returns `false` when stdout is closed.
pub(crate) fn print_header(columns: &[String]) -> bool {
    let cells: Vec<Value> = columns.iter().map(|c| Value::String(c.clone())).collect();
    crate::out!("{}", csv_line(&cells))
}

pub(crate) fn warn_truncated(shown: usize) {
    warn(
        "truncated",
        &format!(
            "more rows match: showing the first {shown}; raise --limit, pass --all, or narrow \
             the query"
        ),
    );
}

/// Search performance rows: clicks, impressions, CTR and average position,
/// grouped by the dimensions you ask for. Google sorts them by clicks,
/// descending.
#[derive(Args, Debug)]
pub(crate) struct Query {
    #[command(flatten)]
    search: SearchArgs,

    /// Sort by a column: clicks, impressions, ctr, position, or one of the
    /// dimensions. Google sorts by clicks only, so this fetches every row
    /// first (up to --max-rows).
    #[arg(long, value_name = "COLUMN")]
    sort: Option<String>,

    /// The --sort direction.
    #[arg(long, value_enum, default_value_t = Order::Desc, requires = "sort")]
    order: Order,

    /// Print at most N rows. Default 1000. A warning says when more match.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), value_name = "N", conflicts_with = "all")]
    limit: Option<u64>,

    /// Print every row. Google keeps at most 50,000 rows a day per search
    /// type.
    #[arg(long)]
    all: bool,

    /// With --sort: the most rows to fetch before sorting.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..), default_value_t = 100_000, value_name = "N")]
    max_rows: u64,
}

impl Leaf for Query {
    fn spec(&self) -> &'static CommandSpec {
        &SPEC
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let spec = self.search.spec();
        let range = self.search.range()?;
        let columns = query_columns(&spec.dimensions);
        if let Some(field) = &self.sort {
            check_sort(field, &columns)?;
        }
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        let cap = match (&self.sort, self.all) {
            (Some(_), _) => Cap::Rows(self.max_rows),
            (None, true) => Cap::All,
            (None, false) => Cap::Rows(limit),
        };
        let site = self.search.site();
        if ctx.dry_run {
            let mut requests = Vec::new();
            if range.needs_latest() {
                requests.push(ctx.api.search_analytics(site, spec.probe(dates::today()))?);
            }
            let mut body = spec.body(range.start, range.end, true);
            let first = match cap {
                Cap::Rows(rows) => (rows + 1).min(PAGE_MAX),
                Cap::All => PAGE_MAX,
            };
            body.insert("rowLimit".into(), first.into());
            body.insert("startRow".into(), 0.into());
            requests.push(ctx.api.search_analytics(site, Value::Object(body))?);
            ctx.print_dry_run(&requests);
            return Ok(());
        }

        let (start, end) = resolve(ctx, site, &spec, &range)?;
        let body = spec.body(Day::Fixed(start), Day::Fixed(end), true);
        let api = ctx.api.clone();
        let format = ctx.format;
        let send = |page| ctx.send(&api.search_analytics(site, page)?);

        if self.sort.is_none() && format != Format::Json {
            // Streams each page as it arrives, and stops when stdout closes
            // (decision 10).
            if format == Format::Csv && !print_header(&columns) {
                return Ok(());
            }
            let paged = fetch(&body, cap, send, |rows| {
                let rows: Vec<Value> = rows.iter().map(|r| named(r, &spec.dimensions)).collect();
                print_rows(&rows, &columns, format)
            })?;
            if paged.truncated && !paged.closed {
                warn_truncated(usize::try_from(paged.rows).unwrap_or(usize::MAX));
            }
            return Ok(());
        }

        let mut rows = Vec::new();
        let paged = fetch(&body, cap, send, |page| {
            rows.extend(page.iter().map(|r| named(r, &spec.dimensions)));
            true
        })?;
        let truncated = match &self.sort {
            Some(field) => {
                if paged.truncated {
                    warn(
                        "row_cap",
                        &format!(
                            "fetched the first {} rows only (--max-rows), so the sort covers \
                             those",
                            self.max_rows
                        ),
                    );
                }
                sort(&mut rows, field, self.order);
                let keep = if self.all {
                    rows.len()
                } else {
                    usize::try_from(limit).unwrap_or(usize::MAX)
                };
                let truncated = rows.len() > keep;
                rows.truncate(keep);
                truncated
            }
            None => paged.truncated,
        };
        if truncated {
            warn_truncated(rows.len());
        }
        if format == Format::Json {
            let mut envelope = Map::new();
            envelope.insert("siteUrl".into(), site.as_str().into());
            envelope.insert("request".into(), Value::Object(body));
            envelope.insert(
                "responseAggregationType".into(),
                paged.aggregation.unwrap_or(Value::Null),
            );
            if let Some(metadata) = paged.metadata {
                envelope.insert("metadata".into(), metadata);
            }
            envelope.insert("rowCount".into(), rows.len().into());
            envelope.insert("truncated".into(), truncated.into());
            envelope.insert("rows".into(), json!(rows));
            output::print(&Value::Object(envelope), Format::Json);
        } else if format == Format::Jsonl || print_header(&columns) {
            print_rows(&rows, &columns, format);
        }
        Ok(())
    }
}
