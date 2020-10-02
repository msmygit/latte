use std::path::PathBuf;
use std::{fs, io};

use err_derive::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::stats::{BenchmarkStats, Percentile, PERCENTILES};
use chrono::{Local, NaiveDateTime, TimeZone};
use core::fmt;
use std::fmt::{Display, Formatter};

/// Keeps all data we want to save in a report:
/// run metadata, configuration and results
#[derive(Serialize, Deserialize)]
pub struct Report {
    pub conf: Config,
    pub percentiles: Vec<f32>,
    pub result: BenchmarkStats,
}

#[derive(Debug, Error)]
pub enum ReportLoadError {
    #[error(display = "{}", _0)]
    IO(#[source] io::Error),
    #[error(display = "{}", _0)]
    Deserialize(#[source] serde_json::Error),
}

/// Saves benchmark results to a JSON file
pub fn save_report(conf: Config, stats: BenchmarkStats, path: &PathBuf) -> io::Result<()> {
    let percentile_legend: Vec<f32> = PERCENTILES.iter().map(|p| p.value() as f32).collect();
    let report = Report {
        conf,
        percentiles: percentile_legend,
        result: stats,
    };
    let f = fs::File::create(path)?;
    serde_json::to_writer_pretty(f, &report)?;
    Ok(())
}

/// Loads benchmark results from a JSON file
pub fn load_report(path: &PathBuf) -> Result<Report, ReportLoadError> {
    let file = fs::File::open(path)?;
    let report = serde_json::from_reader(file)?;
    Ok(report)
}

/// This is similar as the builtin `Option`, but we need it, because the
/// builtin `Option` doesn't implement
/// `Display` and there is no way to do it in this crate
pub enum Maybe<T> {
    None,
    Some(T)
}

impl<T: Display> Display for Maybe<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self {
            Maybe::Some(value) => write!(f, "{}", value),
            Maybe::None => Ok(()),
        }
    }
}

pub struct Quantity<T: Display> {
    pub value: T,
    pub error: Option<T>,
    pub precision: usize,
    pub ratio: Option<f32>,
}

impl<T: Display> Quantity<T> {
    pub fn new(value: T, precision: usize) -> Quantity<T> {
        Quantity {
            value,
            error: None,
            precision,
            ratio: None,
        }
    }

    pub fn with_error(mut self, e: T) -> Self {
        self.error = Some(e);
        self
    }

    pub fn with_ratio(mut self, ratio: f32) -> Self {
        self.ratio = Some(ratio);
        self
    }

    pub fn with_opt_ratio(mut self, ratio: Option<f32>) -> Self {
        self.ratio = ratio;
        self
    }

    fn format_error(&self) -> String {
        match &self.error {
            None => "".to_owned(),
            Some(e) => format!("± {:<6.prec$}", e, prec = self.precision),
        }
    }

    fn format_ratio(&self) -> String {
        match &self.ratio {
            None => "".to_owned(),
            Some(r) => format!("({:5.1}%)", r),
        }
    }
}

impl<T: Display> Display for Quantity<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{value:9.prec$} {error:8} {ratio:8}",
            value = self.value,
            prec = self.precision,
            error = self.format_error(),
            ratio = self.format_ratio()
        )
    }
}

trait Rational {
    fn ratio(a: Self, b: Self) -> Option<f32>;
}

impl Rational for f32 {
    fn ratio(a: Self, b: Self) -> Option<f32> {
        Some(a / b)
    }
}

impl Rational for u64 {
    fn ratio(a: Self, b: Self) -> Option<f32> {
        Some(a as f32 / b as f32)
    }
}

impl Rational for usize {
    fn ratio(a: Self, b: Self) -> Option<f32> {
        Some(a as f32 / b as f32)
    }
}

impl<T: Rational> Rational for Maybe<T> {
    fn ratio(a: Self, b: Self) -> Option<f32> {
        match (a, b) {
            (Maybe::Some(a), Maybe::Some(b)) => Rational::ratio(a, b),
            _ => None
        }
    }
}

impl<T: Rational + Display> Rational for Quantity<T> {
    fn ratio(a: Self, b: Self) -> Option<f32> {
        Rational::ratio(a.value, b.value)
    }
}

impl Rational for String {
    fn ratio(_a: Self, _b: Self) -> Option<f32> {
        None
    }
}

/// A single line of text report
struct Line<M, V, F>
where
    M: Display + Rational,
    F: Fn(V) -> M,
{
    /// Text label
    pub label: String,
    /// Unit of measurement
    pub unit: String,
    /// First object to measure
    pub v1: V,
    /// Second object to measure
    pub v2: Option<V>,
    /// Statistical significance of difference between v1 and v2 (P-value)
    pub p: Option<f32>,
    /// Measurement function
    pub f: F,
}

impl<M, V, F> Line<M, V, F>
where
    M: Display + Rational,
    V: Copy,
    F: Fn(V) -> M,
{
    fn new(label: String, unit: String, v1: V, v2: Option<V>, f: F) -> Self {
        Line {
            label,
            unit,
            v1,
            v2,
            p: None,
            f,
        }
    }

    /// Measures the object `v` by applying `f` to it and formats the measurement result.
    /// If the object is None, returns an empty string.
    fn fmt_measurement(&self, v: Option<V>) -> String {
        v.map(|v| format!("{}", (self.f)(v)))
            .unwrap_or("".to_owned())
    }

    /// Computes the relative difference between v2 and v1 as: 100.0 * f(v2) / f(v1) - 100.0.
    /// Then formats the difference as percentage.
    /// If any of the values are missing, returns an empty String
    fn fmt_relative_diff(&self) -> String {
        self.v2
            .and_then(|v2| {
                let m1 = (self.f)(self.v1);
                let m2 = (self.f)(v2);
                let ratio = Rational::ratio(m1, m2);
                ratio.map(|r| {
                    let diff = 100.0 * (r - 1.0);
                    if !diff.is_nan() {
                        format!("{:+7.1}%", diff)
                    } else {
                        "".to_string()
                    }
                })
            })
            .unwrap_or("".to_string())
    }

    fn fmt_unit(&self) -> String {
        match self.unit.as_str() {
            "" => "".to_string(),
            u => format!("[{}]", u),
        }
    }
}

impl<M, V, F> Display for Line<M, V, F>
where
    M: Display + Rational,
    F: Fn(V) -> M,
    V: Copy,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{label:>19} {unit:>7}: {m1:26} {m2:26} {cmp:8}",
            label = self.label,
            unit = self.fmt_unit(),
            m1 = self.fmt_measurement(Some(self.v1)),
            m2 = self.fmt_measurement(self.v2),
            cmp = self.fmt_relative_diff()
        )
    }
}

pub struct ConfigCmp<'a> {
    pub v1: &'a Config,
    pub v2: Option<&'a Config>,
}

impl ConfigCmp<'_> {
    fn make_line<S, M, F>(&self, label: S, unit: &str, f: F) -> Box<Line<M, &Config, F>>
    where
        S: ToString,
        M: Display + Rational,
        F: Fn(&Config) -> M,
    {
        Box::new(Line::new(
            label.to_string(),
            unit.to_string(),
            self.v1,
            self.v2,
            f,
        ))
    }

    fn format_time(&self, conf: &Config, format: &str) -> String {
        conf
            .timestamp
            .map(|ts| {
                let utc = NaiveDateTime::from_timestamp(ts, 0);
                Local.from_utc_datetime(&utc).format(format).to_string()
            })
            .unwrap_or("".to_string())
    }
}

impl<'a> Display for ConfigCmp<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "CONFIG ==============================================================================================")?;
        writeln!(f, "                             ---------- This ---------- ---------- Other ----------    Change   ")?;

        let lines: Vec<Box<dyn Display>> = vec![
            self.make_line("Date", "", |conf|
                self.format_time(conf, "%a, %d %b %Y")
            ),
            self.make_line("Time", "", |conf|
                self.format_time(conf, "%H:%M:%S %z")
            ),
            self.make_line("Label", "", |conf|
                conf.label.clone().unwrap_or("".to_string())
            ),
            self.make_line("Workload", "", |conf|
                conf.workload.to_string()
            ),
            self.make_line("Threads", "", |conf|
                Quantity::new(conf.threads, 0)
            ),
            self.make_line("Connections", "", |conf|
                Quantity::new(conf.connections, 0)
            ),
            self.make_line("Parallelism limit", "req", |conf|
                Quantity::new(conf.parallelism, 0)
            ),
            self.make_line("Rate limit", "req/s", |conf| {
                match conf.rate {
                    Some(r) => Quantity::new(Maybe::Some(r), 1),
                    None => Quantity::new(Maybe::None, 0)
                }
            }),
            self.make_line("Warmup iterations", "req", |conf|
                Quantity::new(conf.warmup_count, 0)
            ),
            self.make_line("Measured iterations", "req", |conf|
                Quantity::new(conf.count, 0)
            ),
            self.make_line("Sampling", "s", |conf|
                Quantity::new(conf.sampling_period, 1)
            ),
        ];

        for l in lines {
            writeln!(f, "{}", l)?;
        }
        Ok(())
    }
}

pub struct BenchmarkCmp<'a> {
    pub v1: &'a BenchmarkStats,
    pub v2: Option<&'a BenchmarkStats>,
}

impl BenchmarkCmp<'_> {
    fn make_line<S, M, F>(&self, label: S, unit: &str, f: F) -> Box<Line<M, &BenchmarkStats, F>>
    where
        S: ToString,
        M: Display + Rational,
        F: Fn(&BenchmarkStats) -> M,
    {
        Box::new(Line::new(
            label.to_string(),
            unit.to_string(),
            self.v1,
            self.v2,
            f,
        ))
    }
}

/// Formats all benchmark stats
impl<'a> Display for BenchmarkCmp<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "SUMMARY STATS =======================================================================================")?;
        writeln!(f, "                             ---------- This ---------- ---------- Other ----------    Change   ")?;

        let summary: Vec<Box<dyn Display>> = vec![
            self.make_line("Elapsed", "s", |s| Quantity::new(s.elapsed_time_s, 3)),
            self.make_line("CPU time", "s", |s| {
                Quantity::new(s.cpu_time_s, 3).with_ratio(s.cpu_util)
            }),
            self.make_line("Completed", "req", |s| {
                Quantity::new(s.completed_requests, 0).with_ratio(s.completed_ratio)
            }),
            self.make_line("Errors", "req", |s| {
                Quantity::new(s.errors, 0).with_ratio(s.errors_ratio)
            }),
            self.make_line("Partitions", "", |s| Quantity::new(s.partitions, 0)),
            self.make_line("Rows", "", |s| Quantity::new(s.rows, 0)),
            self.make_line("Parallelism", "req", |s| {
                Quantity::new(s.parallelism, 1).with_ratio(s.parallelism_ratio)
            }),
            self.make_line("Throughput", "req/s", |s| {
                Quantity::new(s.throughput, 0)
                    .with_opt_ratio(s.throughput_ratio)
                    .with_error(s.throughput_err)
            }),
            self.make_line("Mean resp. time", "ms", |s| {
                Quantity::new(s.resp_time_ms, 2).with_error(s.resp_time_err)
            }),
        ];

        for l in summary {
            writeln!(f, "{}", l)?;
        }

        writeln!(f)?;
        writeln!(f, "THROUGHPUT [req/s] ==================================================================================")?;
        writeln!(f, "                             ---------- This ---------- ---------- Other ----------    Change   ")?;

        let throughput_percentiles = [
            Percentile::Min,
            Percentile::P1,
            Percentile::P2,
            Percentile::P5,
            Percentile::P10,
            Percentile::P25,
            Percentile::P50,
            Percentile::P75,
            Percentile::P90,
            Percentile::P95,
            Percentile::Max,
        ];
        for p in throughput_percentiles.iter() {
            let l = self.make_line(p.name(), "", |s| {
                Quantity::new(s.throughput_percentiles[*p as usize], 0)
            });
            writeln!(f, "{}", l)?;
        }

        let resp_time_percentiles = [
            Percentile::Min,
            Percentile::P25,
            Percentile::P50,
            Percentile::P75,
            Percentile::P90,
            Percentile::P95,
            Percentile::P98,
            Percentile::P99,
            Percentile::P99_9,
            Percentile::P99_99,
            Percentile::Max,
        ];

        writeln!(f)?;
        writeln!(f, "RESPONSE TIMES [ms] =================================================================================")?;
        writeln!(f, "                             ---------- This ---------- ---------- Other ----------    Change   ")?;

        for p in resp_time_percentiles.iter() {
            let l = self.make_line(p.name(), "", |s| {
                Quantity::new(s.resp_time_percentiles[*p as usize], 2)
                    .with_error(s.resp_time_percentiles_err[*p as usize])
            });
            writeln!(f, "{}", l)?;
        }

        writeln!(f)?;
        writeln!(f, "RESPONSE TIME DISTRIBUTION ==========================================================================")?;
        writeln!(f, "Percentile     Resp. time [ms]  ----------------------------- Count ---------------------------------")?;
        for x in self.v1.resp_time_distribution.iter() {
            writeln!(
                f,
                " {:9.5}     {:9.2}       {:9}   {}",
                x.percentile,
                x.resp_time_ms,
                x.count,
                "*".repeat((100 * x.count / self.v1.completed_requests) as usize)
            )?;
        }
        Ok(())
    }
}
