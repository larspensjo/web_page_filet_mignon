//! Stdout progress reporter for the --refresh-stale-summaries-limit mode.
//!
//! Activated only when both stdout and stderr are terminals; otherwise every
//! method is a no-op.

use crossterm::{
    cursor::{Hide, MoveDown, MoveToColumn, MoveUp, Show},
    terminal::{self, Clear, ClearType},
    QueueableCommand,
};
use std::io::Write;

mod dashboard;
pub(crate) use dashboard::{format_dashboard, ProgressGlyphs, MIN_DASHBOARD_WIDTH};
mod import_reporter;
pub use import_reporter::ImportProgressReporter;
mod projection;
#[allow(unused_imports)]
pub use projection::{
    classify_display_phase, format_local_timestamp, BatchDisplayPhase, BatchProgressProjection,
    BatchProgressSnapshot, BatchRunBaseline, IntakeProgress, PassCounts, ProgressClock,
    ProjectionContext, ProviderLifecycle, ProviderProgress, ProviderStageProgress, StageProgress,
    SystemProgressClock, WaitProgress,
};
mod stale_reporter;
#[allow(unused_imports)]
pub use stale_reporter::{format_eta, ProgressReporter};

/// Cursor-managed stdout surface. It owns only terminal control and a caller
/// supplied writer; its input remains the pure dashboard frame above.
pub struct TerminalProgressSurface<W: Write> {
    sink: W,
    glyphs: ProgressGlyphs,
    enabled: bool,
    painted_lines: usize,
    cursor_hidden: bool,
    finished: bool,
}

impl<W: Write> TerminalProgressSurface<W> {
    pub fn new(sink: W, glyphs: ProgressGlyphs) -> Self {
        Self {
            sink,
            glyphs,
            enabled: true,
            painted_lines: 0,
            cursor_hidden: false,
            finished: false,
        }
    }

    /// Creates an inert surface for callers that intentionally selected plain
    /// output. It emits no terminal-control bytes.
    #[cfg(test)]
    pub fn disabled(sink: W, glyphs: ProgressGlyphs) -> Self {
        Self {
            sink,
            glyphs,
            enabled: false,
            painted_lines: 0,
            cursor_hidden: false,
            finished: false,
        }
    }

    /// Queries the terminal for every repaint so a resize is reflected in the
    /// next frame. Terminal-query failures use the conservative dashboard
    /// minimum rather than propagating a presentation-only failure.
    pub fn repaint(&mut self, snapshot: &BatchProgressSnapshot) -> std::io::Result<()> {
        let width = terminal::size()
            .map(|(columns, _)| usize::from(columns).max(1))
            .unwrap_or(MIN_DASHBOARD_WIDTH);
        self.repaint_with_width(snapshot, width)
    }

    /// Paints at an explicit width. This is useful for deterministic tests and
    /// for any future terminal abstraction; production callers use
    /// [`Self::repaint`] so the width is queried each time.
    pub fn repaint_with_width(
        &mut self,
        snapshot: &BatchProgressSnapshot,
        width: usize,
    ) -> std::io::Result<()> {
        if !self.enabled || self.finished {
            return Ok(());
        }
        if !self.cursor_hidden {
            self.sink.queue(Hide)?;
            self.cursor_hidden = true;
        }
        self.clear_previous_frame()?;
        let lines = format_dashboard(snapshot, width, self.glyphs);
        for (index, line) in lines.iter().enumerate() {
            self.sink.queue(MoveToColumn(0))?;
            self.sink.queue(Clear(ClearType::CurrentLine))?;
            self.sink.write_all(line.as_bytes())?;
            if index + 1 < lines.len() {
                self.sink.write_all(b"\n")?;
            }
        }
        self.sink.flush()?;
        self.painted_lines = lines.len();
        Ok(())
    }

    /// Clears the current dashboard and makes the cursor visible so ordinary
    /// append-only diagnostics can be printed by the caller.
    pub fn suspend_for_output(&mut self) -> std::io::Result<()> {
        if !self.enabled || self.finished {
            return Ok(());
        }
        self.clear_previous_frame()?;
        self.show_cursor()?;
        self.sink.flush()
    }

    /// Hides the cursor again and paints a replacement frame after a caller's
    /// ordinary output has been written.
    pub fn resume(&mut self, snapshot: &BatchProgressSnapshot) -> std::io::Result<()> {
        self.repaint(snapshot)
    }

    /// Restores cursor visibility and terminates the current final frame. The
    /// caller paints the final snapshot before calling this method.
    pub fn finish(&mut self) -> std::io::Result<()> {
        if self.finished {
            return Ok(());
        }
        if self.enabled {
            self.show_cursor()?;
            self.sink.write_all(b"\n")?;
            self.sink.flush()?;
        }
        self.painted_lines = 0;
        self.finished = true;
        Ok(())
    }

    #[cfg(test)]
    pub fn sink(&self) -> &W {
        &self.sink
    }

    fn clear_previous_frame(&mut self) -> std::io::Result<()> {
        if self.painted_lines == 0 {
            return Ok(());
        }
        let previous_lines = terminal_count(self.painted_lines);
        if self.painted_lines > 1 {
            self.sink.queue(MoveUp(previous_lines - 1))?;
        }
        for _ in 0..self.painted_lines {
            self.sink.queue(MoveToColumn(0))?;
            self.sink.queue(Clear(ClearType::CurrentLine))?;
            self.sink.queue(MoveDown(1))?;
        }
        self.sink.queue(MoveUp(previous_lines))?;
        self.painted_lines = 0;
        Ok(())
    }

    fn show_cursor(&mut self) -> std::io::Result<()> {
        if self.cursor_hidden {
            self.sink.queue(Show)?;
            self.cursor_hidden = false;
        }
        Ok(())
    }
}

impl<W: Write> Drop for TerminalProgressSurface<W> {
    fn drop(&mut self) {
        if self.finished || !self.enabled {
            return;
        }
        let _ = self.show_cursor();
        let _ = self.sink.write_all(b"\n");
        let _ = self.sink.flush();
    }
}

fn terminal_count(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Append-only progress sink for redirected output. Its compact rows are
/// deliberately ASCII and contain neither carriage-return dashboards nor
/// cursor-control sequences.
pub struct PlainProgressReporter<W: Write> {
    sink: W,
}

impl<W: Write> PlainProgressReporter<W> {
    pub fn new(sink: W) -> Self {
        Self { sink }
    }

    pub fn report(&mut self, snapshot: &BatchProgressSnapshot) -> std::io::Result<()> {
        let line = format_dashboard(snapshot, MIN_DASHBOARD_WIDTH - 1, ProgressGlyphs::Ascii)
            .into_iter()
            .next()
            .unwrap_or_default();
        writeln!(self.sink, "{line}")?;
        self.sink.flush()
    }

    #[cfg(test)]
    pub fn sink(&self) -> &W {
        &self.sink
    }
}

#[cfg(test)]
mod tests {
    use super::dashboard::renderer_snapshot;
    use super::*;

    #[test]
    fn plain_and_disabled_terminal_surfaces_emit_no_cursor_control_bytes() {
        let snapshot = renderer_snapshot(BatchDisplayPhase::Signals);
        let mut plain = PlainProgressReporter::new(Vec::new());
        plain.report(&snapshot).unwrap();
        let plain = std::str::from_utf8(plain.sink()).unwrap();
        assert!(!plain.contains('\u{1b}') && !plain.contains('\r'));

        let mut disabled = TerminalProgressSurface::disabled(Vec::new(), ProgressGlyphs::Unicode);
        disabled.repaint_with_width(&snapshot, 140).unwrap();
        assert!(disabled.sink().is_empty());
    }

    #[derive(Clone)]
    struct SharedOutput(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for SharedOutput {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn terminal_surface_drop_and_finish_restore_cursor_and_terminate_the_frame() {
        let snapshot = renderer_snapshot(BatchDisplayPhase::Signals);
        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        {
            let mut surface = TerminalProgressSurface::new(shared.clone(), ProgressGlyphs::Unicode);
            surface.repaint_with_width(&snapshot, 140).unwrap();
        }
        let dropped = shared.0.lock().unwrap().clone();
        assert!(dropped.windows(6).any(|bytes| bytes == b"\x1b[?25h"));
        assert!(dropped.ends_with(b"\n"));

        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        let mut surface = TerminalProgressSurface::new(shared.clone(), ProgressGlyphs::Unicode);
        surface.repaint_with_width(&snapshot, 140).unwrap();
        surface.finish().unwrap();
        drop(surface);
        let finished = shared.0.lock().unwrap().clone();
        assert_eq!(
            finished
                .windows(6)
                .filter(|bytes| *bytes == b"\x1b[?25h")
                .count(),
            1
        );
        assert!(finished.ends_with(b"\n"));
    }

    #[test]
    fn terminal_surface_repaint_clears_the_prior_multiline_frame_before_repainting() {
        let mut first = renderer_snapshot(BatchDisplayPhase::Signals);
        first.wait = Some(WaitProgress {
            last_provider_check: None,
            next_provider_check: None,
            checked_age: None,
            countdown: None,
            last_provider_check_local: None,
            next_provider_check_local: None,
            last_provider_check_display: None,
            next_provider_check_display: None,
        });
        let second = renderer_snapshot(BatchDisplayPhase::Complete);
        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        let mut surface = TerminalProgressSurface::new(shared.clone(), ProgressGlyphs::Unicode);

        surface.repaint_with_width(&first, 140).unwrap();
        surface.repaint_with_width(&second, 60).unwrap();

        let output = String::from_utf8(shared.0.lock().unwrap().clone()).unwrap();
        // The first wide dashboard is six rows; the second repaint must move
        // back to its first row and clear every previous row before drawing a
        // one-line narrow fallback.
        assert!(
            output.contains("\u{1b}[5A"),
            "missing MoveUp for prior frame: {output:?}"
        );
        assert!(
            output.matches("\u{1b}[2K").count() >= 7,
            "prior rows were not all cleared"
        );
    }
}
