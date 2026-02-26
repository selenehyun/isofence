use std::fmt;
use std::io::IsTerminal;
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stderr;

/// Progress reporting trait for the analysis pipeline.
pub trait Progress: Send + Sync {
    /// Phase 1&2 starting: file analysis begins.
    fn start_file_analysis(&self, total: u64);
    /// A single file has been analyzed (called from rayon workers).
    fn file_analyzed(&self);
    /// Phase 3 starting: graph analysis begins.
    fn start_graph_phase(&self);
    /// A sub-step within Phase 3.
    fn graph_step(&self, message: &str);
    /// Hazard reachability starting with known total test files.
    fn start_reachability(&self, total: u64);
    /// One test file's reachability computed.
    fn reachability_step(&self);
    /// Analysis complete — clean up any display.
    fn finish(&self);
}

/// Format elapsed duration with ms precision.
fn format_elapsed(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Custom elapsed key for indicatif templates — shows ms precision.
fn elapsed_ms(state: &ProgressState, w: &mut dyn fmt::Write) {
    let _ = write!(w, "{}", format_elapsed(state.elapsed()));
}

/// Interactive terminal progress with spinner + progress bar.
pub struct TerminalProgress {
    bar: Mutex<ProgressBar>,
    file_count: Mutex<u64>,
    test_count: Mutex<u64>,
}

impl TerminalProgress {
    pub fn new() -> Self {
        Self {
            bar: Mutex::new(ProgressBar::hidden()),
            file_count: Mutex::new(0),
            test_count: Mutex::new(0),
        }
    }

    /// Finish current bar, print a completion line with elapsed time, return elapsed.
    fn finish_phase(&self, description: &str) {
        let bar = self.bar.lock().unwrap();
        let elapsed = bar.elapsed();
        bar.finish_and_clear();
        eprintln!(
            "  {} {} ({})",
            "✓".if_supports_color(Stderr, |s| s.green()),
            description,
            format_elapsed(elapsed)
                .if_supports_color(Stderr, |s| s.dimmed()),
        );
    }
}

impl Progress for TerminalProgress {
    fn start_file_analysis(&self, total: u64) {
        *self.file_count.lock().unwrap() = total;
        let bar = ProgressBar::new(total);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} Analyzing files [{pos}/{len}] {bar:20.cyan/dim} {elapsed_ms}",
            )
            .unwrap()
            .with_key("elapsed_ms", elapsed_ms)
            .progress_chars("█▓░"),
        );
        bar.enable_steady_tick(Duration::from_millis(80));
        *self.bar.lock().unwrap() = bar;
    }

    fn file_analyzed(&self) {
        self.bar.lock().unwrap().inc(1);
    }

    fn start_graph_phase(&self) {
        let count = *self.file_count.lock().unwrap();
        self.finish_phase(&format!("Analyzed {} files", count));

        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed_ms}")
                .unwrap()
                .with_key("elapsed_ms", elapsed_ms),
        );
        bar.set_message("Building module graph...");
        bar.enable_steady_tick(Duration::from_millis(80));
        *self.bar.lock().unwrap() = bar;
    }

    fn graph_step(&self, message: &str) {
        self.bar.lock().unwrap().set_message(message.to_string());
    }

    fn start_reachability(&self, total: u64) {
        *self.test_count.lock().unwrap() = total;
        self.finish_phase("Built module graph");

        let bar = ProgressBar::new(total);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} Checking reachability [{pos}/{len}] {bar:20.cyan/dim} {elapsed_ms}",
            )
            .unwrap()
            .with_key("elapsed_ms", elapsed_ms)
            .progress_chars("█▓░"),
        );
        bar.enable_steady_tick(Duration::from_millis(80));
        *self.bar.lock().unwrap() = bar;
    }

    fn reachability_step(&self) {
        self.bar.lock().unwrap().inc(1);
    }

    fn finish(&self) {
        let count = *self.test_count.lock().unwrap();
        self.finish_phase(&format!("Checked reachability for {} test files", count));
    }
}

/// CI / non-TTY progress: simple eprintln lines.
pub struct CiProgress;

impl Progress for CiProgress {
    fn start_file_analysis(&self, total: u64) {
        eprintln!("Analyzing {total} files...");
    }

    fn file_analyzed(&self) {}

    fn start_graph_phase(&self) {
        eprintln!("Building module graph...");
    }

    fn graph_step(&self, message: &str) {
        eprintln!("{message}");
    }

    fn start_reachability(&self, total: u64) {
        eprintln!("Checking reachability for {total} test files...");
    }

    fn reachability_step(&self) {}

    fn finish(&self) {}
}

/// Silent progress: no-op.
pub struct SilentProgress;

impl Progress for SilentProgress {
    fn start_file_analysis(&self, _total: u64) {}
    fn file_analyzed(&self) {}
    fn start_graph_phase(&self) {}
    fn graph_step(&self, _message: &str) {}
    fn start_reachability(&self, _total: u64) {}
    fn reachability_step(&self) {}
    fn finish(&self) {}
}

/// Create the appropriate progress reporter based on environment.
pub fn create_progress(quiet: bool) -> Box<dyn Progress> {
    if quiet {
        return Box::new(SilentProgress);
    }
    if std::io::stderr().is_terminal() {
        Box::new(TerminalProgress::new())
    } else {
        Box::new(CiProgress)
    }
}
