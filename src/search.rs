//! Search Analytics: the request body, the date window, paging, rows with
//! named keys, sorting, and the period comparison (decisions 3, 9, 10, 11).

use crate::dates;
use crate::error::Error;
use clap::ValueEnum;
use serde_json::{Map, Number, Value, json};
use std::cmp::Ordering;
use std::collections::HashMap;
use time::Date;

/// The most rows one request may ask for.
pub const PAGE_MAX: u64 = 25_000;

/// The four numbers of every row, in the API's order.
pub const METRICS: [&str; 4] = ["clicks", "impressions", "ctr", "position"];

/// The columns `compare` adds to the dimensions, in order.
pub const COMPARE_COLUMNS: [&str; 12] = [
    "clicks",
    "clicksPrevious",
    "clicksChange",
    "impressions",
    "impressionsPrevious",
    "impressionsChange",
    "ctr",
    "ctrPrevious",
    "ctrChange",
    "position",
    "positionPrevious",
    "positionChange",
];

/// How many days back the probe for the latest date looks.
const PROBE_DAYS: i64 = 16;

/// A dimension to group rows by. The values are the API's spelling
/// (decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Dimension {
    #[value(name = "date")]
    Date,
    /// With `--data-state hourly_all`.
    #[value(name = "hour")]
    Hour,
    #[value(name = "query")]
    Query,
    #[value(name = "page")]
    Page,
    /// ISO 3166-1 alpha-3, lowercase: `usa`, `deu`.
    #[value(name = "country")]
    Country,
    /// `DESKTOP`, `MOBILE` or `TABLET`.
    #[value(name = "device")]
    Device,
    /// Rich result types, such as `AMP_BLUE_LINK`.
    #[value(name = "searchAppearance")]
    SearchAppearance,
}

impl Dimension {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Dimension::Date => "date",
            Dimension::Hour => "hour",
            Dimension::Query => "query",
            Dimension::Page => "page",
            Dimension::Country => "country",
            Dimension::Device => "device",
            Dimension::SearchAppearance => "searchAppearance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SearchType {
    #[value(name = "web")]
    Web,
    #[value(name = "image")]
    Image,
    #[value(name = "video")]
    Video,
    /// The "News" tab of Google Search.
    #[value(name = "news")]
    News,
    /// No `query` dimension, no position.
    #[value(name = "discover")]
    Discover,
    /// news.google.com and the Google News app. No `query` dimension.
    #[value(name = "googleNews")]
    GoogleNews,
}

impl SearchType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SearchType::Web => "web",
            SearchType::Image => "image",
            SearchType::Video => "video",
            SearchType::News => "news",
            SearchType::Discover => "discover",
            SearchType::GoogleNews => "googleNews",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DataState {
    /// Final data only: about 2-3 days behind.
    #[value(name = "final")]
    Final,
    /// Also the fresh data that may still change.
    #[value(name = "all")]
    All,
    /// Fresh data by hour; use with the `hour` dimension.
    #[value(name = "hourly_all")]
    HourlyAll,
}

impl DataState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DataState::Final => "final",
            DataState::All => "all",
            DataState::HourlyAll => "hourly_all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AggregationType {
    #[value(name = "auto")]
    Auto,
    #[value(name = "byPage")]
    ByPage,
    #[value(name = "byProperty")]
    ByProperty,
    #[value(name = "byNewsShowcasePanel")]
    ByNewsShowcasePanel,
}

impl AggregationType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AggregationType::Auto => "auto",
            AggregationType::ByPage => "byPage",
            AggregationType::ByProperty => "byProperty",
            AggregationType::ByNewsShowcasePanel => "byNewsShowcasePanel",
        }
    }
}

/// The dimensions a filter can test. `date` and `hour` are not filterable:
/// the date range does that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FilterDimension {
    #[value(name = "query")]
    Query,
    #[value(name = "page")]
    Page,
    #[value(name = "country")]
    Country,
    #[value(name = "device")]
    Device,
    #[value(name = "searchAppearance")]
    SearchAppearance,
}

impl FilterDimension {
    fn as_str(self) -> &'static str {
        match self {
            FilterDimension::Query => "query",
            FilterDimension::Page => "page",
            FilterDimension::Country => "country",
            FilterDimension::Device => "device",
            FilterDimension::SearchAppearance => "searchAppearance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Operator {
    #[value(name = "equals")]
    Equals,
    #[value(name = "notEquals")]
    NotEquals,
    #[value(name = "contains")]
    Contains,
    #[value(name = "notContains")]
    NotContains,
    /// RE2 syntax.
    #[value(name = "includingRegex")]
    IncludingRegex,
    /// RE2 syntax.
    #[value(name = "excludingRegex")]
    ExcludingRegex,
}

impl Operator {
    fn as_str(self) -> &'static str {
        match self {
            Operator::Equals => "equals",
            Operator::NotEquals => "notEquals",
            Operator::Contains => "contains",
            Operator::NotContains => "notContains",
            Operator::IncludingRegex => "includingRegex",
            Operator::ExcludingRegex => "excludingRegex",
        }
    }
}

/// One `--filter DIMENSION:OPERATOR:EXPRESSION`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub dimension: FilterDimension,
    pub operator: Operator,
    pub expression: String,
}

impl Filter {
    /// Splits on the first two colons only: a URL or a regex in the
    /// expression keeps its own.
    ///
    /// # Errors
    ///
    /// The text for clap.
    pub fn parse(value: &str) -> Result<Filter, String> {
        let usage = "write DIMENSION:OPERATOR:EXPRESSION, for example page:includingRegex:/blog/";
        let mut parts = value.splitn(3, ':');
        let (Some(dimension), Some(operator), Some(expression)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("{value:?}: {usage}"));
        };
        let names = |values: &[clap::builder::PossibleValue]| {
            values
                .iter()
                .map(|v| v.get_name().to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let dimension = FilterDimension::from_str(dimension, true).map_err(|_| {
            let all: Vec<_> = FilterDimension::value_variants()
                .iter()
                .filter_map(ValueEnum::to_possible_value)
                .collect();
            format!(
                "{dimension:?} is not a filter dimension: use one of {}",
                names(&all)
            )
        })?;
        let operator = Operator::from_str(operator, true).map_err(|_| {
            let all: Vec<_> = Operator::value_variants()
                .iter()
                .filter_map(ValueEnum::to_possible_value)
                .collect();
            format!(
                "{operator:?} is not an operator: use one of {}",
                names(&all)
            )
        })?;
        if expression.is_empty() {
            return Err(format!("{value:?} has no expression: {usage}"));
        }
        Ok(Filter {
            dimension,
            operator,
            expression: expression.to_owned(),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "dimension": self.dimension.as_str(),
            "operator": self.operator.as_str(),
            "expression": self.expression,
        })
    }
}

/// A day of a date range: known, or counted back from the latest day with
/// data, which only the probe can tell (decision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Fixed(Date),
    /// The latest day with data, minus this many days.
    Latest(i64),
    /// Depends on a range whose length is not known before the probe.
    Unknown,
}

impl Day {
    #[must_use]
    pub fn minus(self, days: i64) -> Day {
        match self {
            Day::Fixed(date) => Day::Fixed(dates::minus(date, days)),
            Day::Latest(back) => Day::Latest(back + days),
            Day::Unknown => Day::Unknown,
        }
    }

    /// The date, now that the latest day is known.
    #[must_use]
    pub fn resolve(self, latest: Date) -> Option<Date> {
        match self {
            Day::Fixed(date) => Some(date),
            Day::Latest(back) => Some(dates::minus(latest, back)),
            Day::Unknown => None,
        }
    }

    /// The date, or a placeholder for `--dry-run`.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Day::Fixed(date) => dates::format(date).into(),
            Day::Latest(0) => "<latest day with data>".into(),
            Day::Latest(back) => format!("<latest day with data - {back} days>").into(),
            Day::Unknown => "<known after the latest day with data>".into(),
        }
    }
}

/// A date range, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Day,
    pub end: Day,
}

impl Range {
    /// From `--start-date`, `--end-date` and `--days` (decision 9).
    ///
    /// # Errors
    ///
    /// A `config` error for all three flags at once, or a start after the
    /// end.
    pub fn from_flags(
        start: Option<Date>,
        end: Option<Date>,
        days: Option<u32>,
    ) -> Result<Range, Error> {
        let back = i64::from(days.unwrap_or(28)) - 1;
        match (start, end) {
            (Some(_), Some(_)) if days.is_some() => Err(Error::config(
                "--days goes with --start-date or --end-date, not with both",
            )),
            (Some(start), Some(end)) if start > end => Err(Error::config(format!(
                "--start-date {} is after --end-date {}",
                dates::format(start),
                dates::format(end)
            ))),
            (Some(start), Some(end)) => Ok(Range {
                start: Day::Fixed(start),
                end: Day::Fixed(end),
            }),
            (Some(start), None) if days.is_some() => Ok(Range {
                start: Day::Fixed(start),
                end: Day::Fixed(dates::plus(start, back)),
            }),
            (Some(start), None) => Ok(Range {
                start: Day::Fixed(start),
                end: Day::Latest(0),
            }),
            (None, Some(end)) => Ok(Range {
                start: Day::Fixed(dates::minus(end, back)),
                end: Day::Fixed(end),
            }),
            (None, None) => Ok(Range {
                start: Day::Latest(back),
                end: Day::Latest(0),
            }),
        }
    }

    /// Whether the probe must run first.
    #[must_use]
    pub fn needs_latest(&self) -> bool {
        matches!(self.start, Day::Latest(_)) || matches!(self.end, Day::Latest(_))
    }

    /// Days in the range, when they are known before the probe.
    #[must_use]
    pub fn length(&self) -> Option<i64> {
        match (self.start, self.end) {
            (Day::Fixed(start), Day::Fixed(end)) => Some(dates::length(start, end)),
            (Day::Latest(start), Day::Latest(end)) => Some(start - end + 1),
            _ => None,
        }
    }

    /// The range of the same length just before this one.
    #[must_use]
    pub fn previous(&self) -> Range {
        match self.length() {
            Some(length) => Range {
                start: self.start.minus(length),
                end: self.start.minus(1),
            },
            None => Range {
                start: Day::Unknown,
                end: Day::Unknown,
            },
        }
    }

    /// The same weekdays 52 weeks earlier.
    #[must_use]
    pub fn year_before(&self) -> Range {
        Range {
            start: self.start.minus(364),
            end: self.end.minus(364),
        }
    }

    /// The range of the same length that starts on `start`.
    #[must_use]
    pub fn starting(&self, start: Date) -> Range {
        match self.length() {
            Some(length) => Range {
                start: Day::Fixed(start),
                end: Day::Fixed(dates::plus(start, length - 1)),
            },
            None => Range {
                start: Day::Fixed(start),
                end: Day::Unknown,
            },
        }
    }

    /// Both dates, now that the latest day is known.
    ///
    /// # Errors
    ///
    /// A `config` error when the start is after the resolved end.
    pub fn resolve(&self, latest: Date) -> Result<(Date, Date), Error> {
        let start = self.start.resolve(latest).unwrap_or(latest);
        let end = self.end.resolve(latest).unwrap_or(latest);
        if start > end {
            return Err(Error::config(format!(
                "--start-date {} is after the latest day with data, {}",
                dates::format(start),
                dates::format(end)
            )));
        }
        Ok((start, end))
    }
}

/// Everything about a Search Analytics request except its dates and paging.
#[derive(Debug, Clone)]
pub struct Spec {
    pub dimensions: Vec<Dimension>,
    pub filters: Vec<Filter>,
    pub search_type: SearchType,
    pub data_state: DataState,
    pub aggregation_type: Option<AggregationType>,
}

impl Spec {
    /// The request body, in the order of Google's reference, without
    /// `rowLimit` and `startRow` (paging adds them). `grouped` is false for
    /// the totals request, which has no dimensions.
    #[must_use]
    pub fn body(&self, start: Day, end: Day, grouped: bool) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("startDate".into(), start.to_json());
        body.insert("endDate".into(), end.to_json());
        if grouped && !self.dimensions.is_empty() {
            let dimensions: Vec<&str> = self.dimensions.iter().map(|d| d.as_str()).collect();
            body.insert("dimensions".into(), json!(dimensions));
        }
        body.insert("type".into(), self.search_type.as_str().into());
        if !self.filters.is_empty() {
            let filters: Vec<Value> = self.filters.iter().map(Filter::to_json).collect();
            body.insert(
                "dimensionFilterGroups".into(),
                json!([{"groupType": "and", "filters": filters}]),
            );
        }
        if let Some(aggregation) = self.aggregation_type {
            body.insert("aggregationType".into(), aggregation.as_str().into());
        }
        body.insert("dataState".into(), self.data_state.as_str().into());
        body
    }

    /// The request that finds the latest day with data: daily totals for the
    /// last 16 days. Hourly data is grouped by hour only, so the probe asks
    /// for `all` instead (decision 9).
    #[must_use]
    pub fn probe(&self, today: Date) -> Value {
        let state = match self.data_state {
            DataState::HourlyAll => DataState::All,
            other => other,
        };
        json!({
            "startDate": dates::format(dates::minus(today, PROBE_DAYS)),
            "endDate": dates::format(dates::minus(today, 1)),
            "dimensions": ["date"],
            "type": self.search_type.as_str(),
            "dataState": state.as_str(),
            "rowLimit": PROBE_DAYS,
        })
    }
}

/// The latest date in a probe answer, or `None` when it has no rows.
#[must_use]
pub fn latest_in(probe: &Value) -> Option<Date> {
    rows_of(probe)
        .iter()
        .filter_map(|row| row.pointer("/keys/0").and_then(Value::as_str))
        .filter_map(|text| dates::parse(text).ok())
        .max()
}

/// The `rows` of an answer. Google leaves the key out when there are none.
#[must_use]
pub fn rows_of(answer: &Value) -> &[Value] {
    answer
        .get("rows")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// How many rows to fetch (decision 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    /// At most this many. One more is asked for, to tell whether rows were
    /// left out.
    Rows(u64),
    /// Every row Google gives.
    All,
}

/// What a paged fetch saw besides the rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paged {
    /// `responseAggregationType` of the first page.
    pub aggregation: Option<Value>,
    /// `metadata` of the first page: where incomplete data starts.
    pub metadata: Option<Value>,
    /// More rows match than the cap let through.
    pub truncated: bool,
    /// stdout closed, so the fetch stopped early.
    pub closed: bool,
    /// Rows handed to the sink.
    pub rows: u64,
}

/// Fetches every page of `body` up to `cap`, one request at a time, and hands
/// each page's rows to `sink`. `sink` returns `false` when stdout is closed,
/// which stops the fetch.
///
/// # Errors
///
/// The first failed request's error.
pub fn fetch(
    body: &Map<String, Value>,
    cap: Cap,
    mut send: impl FnMut(Value) -> Result<Value, Error>,
    mut sink: impl FnMut(&[Value]) -> bool,
) -> Result<Paged, Error> {
    let wanted = match cap {
        Cap::Rows(limit) => Some(limit + 1),
        Cap::All => None,
    };
    let mut paged = Paged::default();
    let mut fetched: u64 = 0;
    loop {
        let size = wanted.map_or(PAGE_MAX, |wanted| (wanted - fetched).min(PAGE_MAX));
        let mut page = body.clone();
        page.insert("rowLimit".into(), size.into());
        page.insert("startRow".into(), fetched.into());
        let answer = send(Value::Object(page))?;
        if fetched == 0 {
            paged.aggregation = answer.get("responseAggregationType").cloned();
            paged.metadata = answer.get("metadata").cloned();
        }
        let rows = rows_of(&answer);
        let count = rows.len() as u64;
        fetched += count;
        let keep = match cap {
            Cap::Rows(limit) => {
                let room = limit.saturating_sub(paged.rows);
                paged.truncated = fetched > limit;
                usize::try_from(room).map_or(rows.len(), |room| room.min(rows.len()))
            }
            Cap::All => rows.len(),
        };
        if keep > 0 {
            paged.rows += keep as u64;
            if !sink(&rows[..keep]) {
                paged.closed = true;
                return Ok(paged);
            }
        }
        let done = count < size || wanted.is_some_and(|wanted| fetched >= wanted);
        if done {
            return Ok(paged);
        }
    }
}

/// A row with named keys: `{"page": ..., "query": ..., "clicks": ...}`
/// instead of `{"keys": [...], "clicks": ...}` (decision 3).
#[must_use]
pub fn named(row: &Value, dimensions: &[Dimension]) -> Value {
    let mut named = keys_of(row, dimensions);
    if let Some(fields) = row.as_object() {
        for (key, value) in fields {
            if key != "keys" {
                named.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(named)
}

fn keys_of(row: &Value, dimensions: &[Dimension]) -> Map<String, Value> {
    let keys = row.get("keys").and_then(Value::as_array);
    dimensions
        .iter()
        .enumerate()
        .map(|(i, dimension)| {
            let key = keys.and_then(|keys| keys.get(i)).cloned();
            (dimension.as_str().to_owned(), key.unwrap_or(Value::Null))
        })
        .collect()
}

/// The CSV columns of a `query` row.
#[must_use]
pub fn query_columns(dimensions: &[Dimension]) -> Vec<String> {
    dimensions
        .iter()
        .map(|d| d.as_str())
        .chain(METRICS)
        .map(str::to_owned)
        .collect()
}

/// The CSV columns of a `compare` row.
#[must_use]
pub fn compare_columns(dimensions: &[Dimension]) -> Vec<String> {
    dimensions
        .iter()
        .map(|d| d.as_str())
        .chain(COMPARE_COLUMNS)
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Order {
    Asc,
    Desc,
}

/// Sorts rows by one field. Numbers compare as numbers, strings as text,
/// and a row without the field (or with `null`) goes last in either order.
pub fn sort(rows: &mut [Value], field: &str, order: Order) {
    rows.sort_by(|a, b| {
        let (a, b) = (a.get(field), b.get(field));
        match (present(a), present(b)) {
            (Some(a), Some(b)) => {
                let ordering = compare_values(a, b);
                match order {
                    Order::Asc => ordering,
                    Order::Desc => ordering.reverse(),
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    });
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        _ => a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")),
    }
}

/// Joins two periods' rows on their keys (decision 11): one row per key seen
/// in either period, current rows first in Google's order.
#[must_use]
pub fn compare_rows(current: &[Value], previous: &[Value], dimensions: &[Dimension]) -> Vec<Value> {
    let key = |row: &Value| row.get("keys").map(Value::to_string).unwrap_or_default();
    let previous_by_key: HashMap<String, &Value> =
        previous.iter().map(|row| (key(row), row)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::with_capacity(current.len());
    for row in current {
        seen.insert(key(row));
        rows.push(compared(
            keys_of(row, dimensions),
            Some(row),
            previous_by_key.get(&key(row)).copied(),
        ));
    }
    for row in previous {
        if !seen.contains(&key(row)) {
            rows.push(compared(keys_of(row, dimensions), None, Some(row)));
        }
    }
    rows
}

/// The totals of both periods as one compared row. A period without rows has
/// no clicks.
#[must_use]
pub fn compare_totals(current: &Value, previous: &Value) -> Value {
    compared(
        Map::new(),
        rows_of(current).first(),
        rows_of(previous).first(),
    )
}

/// One row of `COMPARE_COLUMNS`. Clicks and impressions missing from a
/// period are 0; CTR and position are `null`, and so is their change.
fn compared(
    mut row: Map<String, Value>,
    current: Option<&Value>,
    previous: Option<&Value>,
) -> Value {
    let get = |side: Option<&Value>, metric: &str| {
        side.and_then(|r| r.get(metric))
            .filter(|v| v.is_number())
            .cloned()
    };
    for metric in ["clicks", "impressions"] {
        let now = get(current, metric).unwrap_or_else(|| 0.into());
        let before = get(previous, metric).unwrap_or_else(|| 0.into());
        let change = number(now.as_f64().unwrap_or(0.0) - before.as_f64().unwrap_or(0.0));
        row.insert(metric.to_owned(), now);
        row.insert(format!("{metric}Previous"), before);
        row.insert(format!("{metric}Change"), change);
    }
    for metric in ["ctr", "position"] {
        let now = get(current, metric);
        let before = get(previous, metric);
        let change = match (
            now.as_ref().and_then(Value::as_f64),
            before.as_ref().and_then(Value::as_f64),
        ) {
            (Some(now), Some(before)) => number(now - before),
            _ => Value::Null,
        };
        row.insert(metric.to_owned(), now.unwrap_or(Value::Null));
        row.insert(format!("{metric}Previous"), before.unwrap_or(Value::Null));
        row.insert(format!("{metric}Change"), change);
    }
    Value::Object(row)
}

/// A computed number, rounded to 6 decimals so that `0.1 - 0.3` prints as
/// `-0.2`, and whole numbers print without `.0`.
#[must_use]
pub fn number(value: f64) -> Value {
    let rounded = (value * 1e6).round() / 1e6;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "only whole numbers well inside the i64 range take this path"
    )]
    if rounded.fract() == 0.0 && rounded.abs() < 9e15 {
        return (rounded as i64).into();
    }
    Number::from_f64(rounded).map_or(Value::Null, Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn spec() -> Spec {
        Spec {
            dimensions: vec![Dimension::Page, Dimension::Query],
            filters: vec![],
            search_type: SearchType::Web,
            data_state: DataState::Final,
            aggregation_type: None,
        }
    }

    fn row(keys: &[&str], clicks: u64, impressions: u64, ctr: f64, position: f64) -> Value {
        json!({"keys": keys, "clicks": clicks, "impressions": impressions, "ctr": ctr, "position": position})
    }

    /// `as_str` is what the body sends, and clap's name is what the user
    /// types: decision 8 says they are the same API spelling.
    fn same_spelling<T: ValueEnum>(as_str: impl Fn(&T) -> &'static str) {
        for variant in T::value_variants() {
            let name = variant.to_possible_value().unwrap();
            assert_eq!(as_str(variant), name.get_name());
        }
    }

    #[test]
    fn every_enum_sends_the_spelling_clap_accepts() {
        same_spelling::<Dimension>(|v| v.as_str());
        same_spelling::<SearchType>(|v| v.as_str());
        same_spelling::<DataState>(|v| v.as_str());
        same_spelling::<AggregationType>(|v| v.as_str());
        same_spelling::<FilterDimension>(|v| v.as_str());
        same_spelling::<Operator>(|v| v.as_str());
    }

    #[test]
    fn a_filter_splits_on_the_first_two_colons() {
        let filter = Filter::parse("PAGE:includingregex:^https://example.com/(a|b):x").unwrap();
        assert_eq!(filter.dimension, FilterDimension::Page);
        assert_eq!(filter.operator, Operator::IncludingRegex);
        assert_eq!(filter.expression, "^https://example.com/(a|b):x");
        assert_eq!(
            filter.to_json(),
            json!({"dimension": "page", "operator": "includingRegex", "expression": "^https://example.com/(a|b):x"})
        );
    }

    #[test]
    fn a_bad_filter_says_what_is_allowed() {
        let cases = [
            ("page", "DIMENSION:OPERATOR:EXPRESSION"),
            ("date:equals:2026-01-01", "searchAppearance"),
            ("page:like:x", "includingRegex"),
            ("page:equals:", "has no expression"),
        ];
        for (value, needle) in cases {
            let error = Filter::parse(value).unwrap_err();
            assert!(error.contains(needle), "{value}: {error}");
        }
    }

    #[test]
    fn the_body_follows_googles_order_and_leaves_out_what_is_unset() {
        let mut spec = spec();
        spec.filters = vec![Filter::parse("query:notContains:brand").unwrap()];
        spec.aggregation_type = Some(AggregationType::ByPage);
        let body = spec.body(
            Day::Fixed(date!(2026 - 01 - 01)),
            Day::Fixed(date!(2026 - 01 - 28)),
            true,
        );
        assert_eq!(
            Value::Object(body).to_string(),
            r#"{"startDate":"2026-01-01","endDate":"2026-01-28","dimensions":["page","query"],"type":"web","dimensionFilterGroups":[{"groupType":"and","filters":[{"dimension":"query","operator":"notContains","expression":"brand"}]}],"aggregationType":"byPage","dataState":"final"}"#
        );
        let totals = spec.body(Day::Latest(27), Day::Latest(0), false);
        assert!(totals.get("dimensions").is_none());
        assert_eq!(totals["startDate"], "<latest day with data - 27 days>");
        assert_eq!(totals["endDate"], "<latest day with data>");
    }

    #[test]
    fn the_probe_asks_for_daily_totals_and_never_hourly() {
        let mut spec = spec();
        spec.data_state = DataState::HourlyAll;
        spec.search_type = SearchType::Discover;
        assert_eq!(
            spec.probe(date!(2026 - 11 - 08)),
            json!({"startDate": "2026-10-23", "endDate": "2026-11-07", "dimensions": ["date"], "type": "discover", "dataState": "all", "rowLimit": 16})
        );
        let answer =
            json!({"rows": [{"keys": ["2026-11-04"]}, {"keys": ["2026-11-05"]}, {"keys": ["x"]}]});
        assert_eq!(latest_in(&answer), Some(date!(2026 - 11 - 05)));
        assert_eq!(latest_in(&json!({})), None);
    }

    #[test]
    fn the_range_flags() {
        let d = |text| dates::parse(text).unwrap();
        let fixed = |a, b| Range {
            start: Day::Fixed(d(a)),
            end: Day::Fixed(d(b)),
        };
        assert_eq!(
            Range::from_flags(None, None, None).unwrap(),
            Range {
                start: Day::Latest(27),
                end: Day::Latest(0)
            }
        );
        assert_eq!(
            Range::from_flags(None, Some(d("2026-01-31")), Some(7)).unwrap(),
            fixed("2026-01-25", "2026-01-31")
        );
        assert_eq!(
            Range::from_flags(Some(d("2026-01-01")), None, Some(7)).unwrap(),
            fixed("2026-01-01", "2026-01-07")
        );
        assert_eq!(
            Range::from_flags(Some(d("2026-01-01")), None, None).unwrap(),
            Range {
                start: Day::Fixed(d("2026-01-01")),
                end: Day::Latest(0)
            }
        );
        assert_eq!(
            Range::from_flags(Some(d("2026-01-01")), Some(d("2026-01-01")), None).unwrap(),
            fixed("2026-01-01", "2026-01-01")
        );
        let both = Range::from_flags(Some(d("2026-01-01")), Some(d("2026-01-02")), Some(2));
        assert!(both.unwrap_err().message().contains("not with both"));
        let backwards = Range::from_flags(Some(d("2026-01-02")), Some(d("2026-01-01")), None);
        assert!(backwards.unwrap_err().message().contains("is after"));
    }

    #[test]
    fn the_previous_periods() {
        let d = |text| dates::parse(text).unwrap();
        let current =
            Range::from_flags(Some(d("2026-02-01")), Some(d("2026-02-28")), None).unwrap();
        assert_eq!(
            current.previous(),
            Range {
                start: Day::Fixed(d("2026-01-04")),
                end: Day::Fixed(d("2026-01-31"))
            }
        );
        assert_eq!(
            current.year_before(),
            Range {
                start: Day::Fixed(d("2025-02-02")),
                end: Day::Fixed(d("2025-03-01"))
            }
        );
        assert_eq!(
            current.starting(d("2025-12-01")),
            Range {
                start: Day::Fixed(d("2025-12-01")),
                end: Day::Fixed(d("2025-12-28"))
            }
        );
        let latest = Range::from_flags(None, None, Some(7)).unwrap();
        assert_eq!(
            latest.previous(),
            Range {
                start: Day::Latest(13),
                end: Day::Latest(7)
            }
        );
        let open = Range::from_flags(Some(d("2026-01-01")), None, None).unwrap();
        assert_eq!(open.length(), None);
        assert_eq!(open.previous().start, Day::Unknown);
        assert_eq!(open.starting(d("2025-01-01")).end, Day::Unknown);
        assert_eq!(Day::Unknown.minus(1), Day::Unknown);
        assert_eq!(Day::Unknown.resolve(d("2026-01-01")), None);
    }

    #[test]
    fn resolving_against_the_latest_day() {
        let d = |text| dates::parse(text).unwrap();
        let latest = Range::from_flags(None, None, Some(7)).unwrap();
        assert_eq!(
            latest.resolve(d("2026-11-05")).unwrap(),
            (d("2026-10-30"), d("2026-11-05"))
        );
        let late = Range::from_flags(Some(d("2026-12-01")), None, None).unwrap();
        assert!(
            late.resolve(d("2026-11-05"))
                .unwrap_err()
                .message()
                .contains("latest day")
        );
    }

    #[test]
    fn fetch_pages_until_a_short_page() {
        let mut sent = Vec::new();
        let mut seen = 0;
        let sizes = [25_000, 3];
        let paged = fetch(
            &Map::new(),
            Cap::All,
            |body| {
                let n = sizes[sent.len()];
                sent.push(body);
                let rows: Vec<Value> = (0..n).map(|_| json!({"keys": ["x"]})).collect();
                Ok(json!({"rows": rows, "responseAggregationType": "byProperty", "metadata": {"first_incomplete_date": "2026-11-07"}}))
            },
            |rows| {
                seen += rows.len();
                true
            },
        )
        .unwrap();
        assert_eq!(seen, 25_003);
        assert_eq!(sent[0]["rowLimit"], 25_000);
        assert_eq!(sent[1]["startRow"], 25_000);
        assert_eq!(paged.aggregation, Some(json!("byProperty")));
        assert_eq!(
            paged.metadata.unwrap()["first_incomplete_date"],
            "2026-11-07"
        );
        assert!(!paged.truncated);
    }

    #[test]
    fn a_cap_asks_for_one_more_row_to_spot_truncation() {
        let answer = |n: usize| -> Result<Value, Error> {
            let rows: Vec<Value> = (0..n).map(|_| json!({})).collect();
            Ok(json!({"rows": rows}))
        };
        let mut limits = Vec::new();
        let mut seen = 0;
        let paged = fetch(
            &Map::new(),
            Cap::Rows(10),
            |body| {
                limits.push(body["rowLimit"].as_u64().unwrap());
                answer(11)
            },
            |rows| {
                seen += rows.len();
                true
            },
        )
        .unwrap();
        assert_eq!(
            (limits, seen, paged.truncated, paged.rows),
            (vec![11], 10, true, 10)
        );

        let exact = fetch(&Map::new(), Cap::Rows(10), |_| answer(10), |_| true).unwrap();
        assert!(!exact.truncated);
        let mut sunk = false;
        let empty = fetch(
            &Map::new(),
            Cap::Rows(10),
            |_| Ok(json!({})),
            |_| {
                sunk = true;
                true
            },
        )
        .unwrap();
        assert!(!sunk, "no rows, no sink");
        assert_eq!(empty.rows, 0);

        // A cap over one page: the second page asks only for what is left.
        let mut limits = Vec::new();
        fetch(
            &Map::new(),
            Cap::Rows(25_000),
            |body| {
                let size = body["rowLimit"].as_u64().unwrap();
                limits.push(size);
                answer(usize::try_from(size).unwrap())
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(limits, [25_000, 1]);
    }

    #[test]
    fn a_closed_stdout_stops_the_fetch() {
        let mut calls = 0;
        let paged = fetch(
            &Map::new(),
            Cap::All,
            |_| {
                calls += 1;
                let rows: Vec<Value> = (0..25_000).map(|_| json!({})).collect();
                Ok(json!({"rows": rows}))
            },
            |_| false,
        )
        .unwrap();
        assert!(paged.closed);
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_failed_page_is_the_error() {
        let error = fetch(&Map::new(), Cap::All, |_| Err(Error::timeout(1)), |_| true).unwrap_err();
        assert_eq!(error.kind(), crate::error::Kind::Timeout);
    }

    #[test]
    fn rows_get_named_keys() {
        let row = row(&["https://example.com/", "shoes"], 3, 40, 0.075, 2.5);
        assert_eq!(
            named(&row, &[Dimension::Page, Dimension::Query]).to_string(),
            r#"{"page":"https://example.com/","query":"shoes","clicks":3,"impressions":40,"ctr":0.075,"position":2.5}"#
        );
        assert_eq!(
            named(&json!({"keys": []}), &[Dimension::Page]),
            json!({"page": null})
        );
        assert_eq!(
            query_columns(&[Dimension::Date]),
            ["date", "clicks", "impressions", "ctr", "position"]
        );
        assert_eq!(compare_columns(&[]).len(), 12);
    }

    #[test]
    fn sort_puts_missing_values_last_in_both_orders() {
        let mut rows = vec![
            json!({"k": 2}),
            json!({"k": null}),
            json!({"k": 10}),
            json!({}),
            json!({"k": 1}),
        ];
        sort(&mut rows, "k", Order::Desc);
        let keys: Vec<&Value> = rows
            .iter()
            .map(|r| r.get("k").unwrap_or(&Value::Null))
            .collect();
        assert_eq!(
            keys,
            [&json!(10), &json!(2), &json!(1), &Value::Null, &Value::Null]
        );
        sort(&mut rows, "k", Order::Asc);
        assert_eq!(rows[0]["k"], 1);
        let mut words = vec![json!({"q": "b"}), json!({"q": "a"})];
        sort(&mut words, "q", Order::Asc);
        assert_eq!(words[0]["q"], "a");
    }

    #[test]
    fn compare_joins_on_keys_and_fills_the_gaps() {
        let current = [
            row(&["/a"], 10, 100, 0.1, 3.0),
            row(&["/new"], 1, 5, 0.2, 8.0),
        ];
        let previous = [
            row(&["/a"], 4, 50, 0.08, 4.5),
            row(&["/gone"], 7, 70, 0.1, 2.0),
        ];
        let rows = compare_rows(&current, &previous, &[Dimension::Page]);
        assert_eq!(
            rows[0].to_string(),
            r#"{"page":"/a","clicks":10,"clicksPrevious":4,"clicksChange":6,"impressions":100,"impressionsPrevious":50,"impressionsChange":50,"ctr":0.1,"ctrPrevious":0.08,"ctrChange":0.02,"position":3.0,"positionPrevious":4.5,"positionChange":-1.5}"#
        );
        assert_eq!(rows[1]["page"], "/new");
        assert_eq!(rows[1]["clicksPrevious"], 0);
        assert_eq!(rows[1]["ctrPrevious"], Value::Null);
        assert_eq!(rows[1]["ctrChange"], Value::Null);
        assert_eq!(rows[2]["page"], "/gone");
        assert_eq!(rows[2]["clicks"], 0);
        assert_eq!(rows[2]["clicksChange"], -7);
        assert_eq!(rows[2]["position"], Value::Null);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn totals_compare_the_single_rows() {
        let current =
            json!({"rows": [{"clicks": 5, "impressions": 9, "ctr": 0.5, "position": 1.0}]});
        let totals = compare_totals(&current, &json!({}));
        assert_eq!(totals["clicksChange"], 5);
        assert_eq!(totals["positionPrevious"], Value::Null);
        let keys: Vec<&String> = totals.as_object().unwrap().keys().collect();
        assert_eq!(keys, COMPARE_COLUMNS.iter().collect::<Vec<_>>());
    }

    #[test]
    fn computed_numbers_are_rounded_and_whole_ones_are_integers() {
        assert_eq!(number(0.1 - 0.3), json!(-0.2));
        assert_eq!(number(6.0), json!(6));
        assert_eq!(number(-1.5), json!(-1.5));
        assert_eq!(number(f64::NAN), Value::Null);
        assert_eq!(number(1e300), json!(1e300));
    }
}
