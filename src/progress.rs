use std::io::IsTerminal;
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

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

/// Interactive terminal progress with spinner + progress bar.
pub struct TerminalProgress {
    bar: Mutex<ProgressBar>,
}

impl TerminalProgress {
    pub fn new() -> Self {
        Self {
            bar: Mutex::new(ProgressBar::hidden()),
        }
    }
}

impl Progress for TerminalProgress {
    fn start_file_analysis(&self, total: u64) {
        let bar = ProgressBar::new(total);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} Analyzing files [{pos}/{len}] {bar:20.cyan/dim} {elapsed}",
            )
            .unwrap()
            .progress_chars("█▓░"),
        );
        bar.enable_steady_tick(Duration::from_millis(100));
        *self.bar.lock().unwrap() = bar;
    }

    fn file_analyzed(&self) {
        self.bar.lock().unwrap().inc(1);
    }

    fn start_graph_phase(&self) {
        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed}")
                .unwrap(),
        );
        bar.set_message("Building module graph...");
        bar.enable_steady_tick(Duration::from_millis(100));
        let old = std::mem::replace(&mut *self.bar.lock().unwrap(), bar);
        old.finish_and_clear();
    }

    fn graph_step(&self, message: &str) {
        self.bar.lock().unwrap().set_message(message.to_string());
    }

    fn start_reachability(&self, total: u64) {
        let bar = ProgressBar::new(total);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} Checking reachability [{pos}/{len}] {bar:20.cyan/dim} {elapsed}",
            )
            .unwrap()
            .progress_chars("█▓░"),
        );
        bar.enable_steady_tick(Duration::from_millis(100));
        let old = std::mem::replace(&mut *self.bar.lock().unwrap(), bar);
        old.finish_and_clear();
    }

    fn reachability_step(&self) {
        self.bar.lock().unwrap().inc(1);
    }

    fn finish(&self) {
        self.bar.lock().unwrap().finish_and_clear();
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
