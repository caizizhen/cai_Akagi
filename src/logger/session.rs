use crate::logger::{binary::BinaryLogger, flow::FlowLogger};
use anyhow::{Context, Result};
use chrono::Local;
use std::{
    collections::HashMap,
    fmt as stdfmt,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::Targets,
    fmt::{
        self,
        format::{DefaultFields, Writer},
        time::{ChronoLocal, FormatTime},
        FmtContext, FormatEvent, FormatFields,
    },
    layer::SubscriberExt,
    registry::LookupSpan,
    util::SubscriberInitExt,
    EnvFilter, Layer, Registry,
};

/// Compact event formatter for file outputs.
///
/// Renders `TIMESTAMP LEVEL target file:line: fields` — deliberately drops
/// the span ancestry list that the default `Full` formatter prefixes onto
/// every event. Third-party crates (e.g. `hudsucker`) wrap our handlers in
/// nested `#[instrument]` spans, producing prefixes longer than the actual
/// message; we don't need them in the file.
struct CompactNoSpans {
    timer: ChronoLocal,
}

impl<S, N> FormatEvent<S, N> for CompactNoSpans
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> stdfmt::Result {
        self.timer.format_time(&mut writer)?;
        let meta = event.metadata();
        write!(writer, " {:>5} {}", meta.level(), meta.target())?;
        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            write!(writer, " {}:{}", file, line)?;
        }
        write!(writer, ": ")?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// One named target → one log file. Events whose `tracing` target matches
/// `prefix` (longest-prefix rule) are also written to `<name>.log`.
#[derive(Debug, Clone)]
pub struct LogTarget {
    pub name: String,
    pub prefix: String,
}

impl LogTarget {
    pub fn new(name: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            prefix: prefix.into(),
        }
    }
}

/// Active logging session. Holds the session directory, registered binary
/// loggers, and tracing-appender worker guards (must stay alive for log
/// flushing).
pub struct Session {
    dir: PathBuf,
    binary_loggers: RwLock<HashMap<String, Arc<BinaryLogger>>>,
    _guards: Vec<WorkerGuard>,
}

impl Session {
    pub fn init(
        log_root: &Path,
        default_level: &str,
        all_level: &str,
        targets: &[LogTarget],
    ) -> Result<Self> {
        let ts = Local::now().format("%Y%m%d-%H%M%S").to_string();
        let dir = log_root.join(&ts);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create log session dir {}", dir.display()))?;

        let timer_fmt = "%Y-%m-%d %H:%M:%S%.3f".to_string();
        let mut guards: Vec<WorkerGuard> = Vec::new();
        let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();

        // Console (stderr): env-controlled level, ANSI on.
        let env_filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(default_level))
            .unwrap_or_else(|_| EnvFilter::new("info"));
        layers.push(
            fmt::layer()
                .with_timer(ChronoLocal::new(timer_fmt.clone()))
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .with_writer(std::io::stderr)
                .with_filter(env_filter)
                .boxed(),
        );

        // Combined all.log — captures every event regardless of target.
        let all_path = dir.join("all.log");
        let all_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&all_path)
            .with_context(|| format!("Failed to open {}", all_path.display()))?;
        let (all_writer, all_guard) = tracing_appender::non_blocking(all_file);
        guards.push(all_guard);
        let all_filter = EnvFilter::try_new(all_level).unwrap_or_else(|_| EnvFilter::new("info"));
        layers.push(
            fmt::layer()
                .event_format(CompactNoSpans {
                    timer: ChronoLocal::new(timer_fmt.clone()),
                })
                .fmt_fields(DefaultFields::new())
                .with_ansi(false)
                .with_writer(all_writer)
                .with_filter(all_filter)
                .boxed(),
        );

        // Per-target files.
        for t in targets {
            let path = dir.join(format!("{}.log", t.name));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("Failed to open {}", path.display()))?;
            let (writer, guard) = tracing_appender::non_blocking(file);
            guards.push(guard);
            let filter = Targets::new().with_target(t.prefix.clone(), tracing::Level::TRACE);
            layers.push(
                fmt::layer()
                    .event_format(CompactNoSpans {
                        timer: ChronoLocal::new(timer_fmt.clone()),
                    })
                    .fmt_fields(DefaultFields::new())
                    .with_ansi(false)
                    .with_writer(writer)
                    .with_filter(filter)
                    .boxed(),
            );
        }

        tracing_subscriber::registry()
            .with(layers)
            .try_init()
            .context("Failed to install tracing subscriber")?;

        Ok(Self {
            dir,
            binary_loggers: RwLock::new(HashMap::new()),
            _guards: guards,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Create a fresh flow logger writing to
    /// `<session>/<subdir>/<file_name>`. The caller supplies the full
    /// filename (extension included). Each call opens a new file handle;
    /// callers (e.g. one per WebSocket flow) own the returned `Arc` and
    /// drop it when the flow ends.
    pub fn flow_logger(
        &self,
        subdir: &str,
        file_name: &str,
        label: impl Into<String>,
    ) -> Result<Arc<FlowLogger>> {
        Ok(Arc::new(FlowLogger::new(
            &self.dir, subdir, file_name, label,
        )?))
    }

    /// Get-or-create a binary logger by name. The file lives at
    /// `<session>/<name>.binlog`.
    pub fn binary_logger(&self, name: &str) -> Result<Arc<BinaryLogger>> {
        if let Some(existing) = self
            .binary_loggers
            .read()
            .expect("binary logger map poisoned")
            .get(name)
        {
            return Ok(existing.clone());
        }
        let mut w = self
            .binary_loggers
            .write()
            .expect("binary logger map poisoned");
        if let Some(existing) = w.get(name) {
            return Ok(existing.clone());
        }
        let logger = Arc::new(BinaryLogger::new(&self.dir, name)?);
        w.insert(name.to_string(), logger.clone());
        Ok(logger)
    }
}
