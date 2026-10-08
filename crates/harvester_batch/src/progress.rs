//! Terminal progress surfaces for article processing and import.
//!
//! Terminal blocks redraw in place; redirected output uses append-only plain lines.

use crossterm::{
    cursor::{Hide, MoveDown, MoveToColumn, MoveUp, Show},
    terminal::{self, Clear, ClearType},
    QueueableCommand,
};
use std::io::Write;

mod block;
#[cfg(test)]
pub(crate) use block::{format_cost, test_stages};
pub(crate) use block::{format_progress_block, progress_status_signature};
mod import_reporter;
pub use import_reporter::ImportProgressReporter;
/// Cursor-managed stdout surface. It owns only terminal control and a caller
/// supplied writer; its input remains the pure block frame above.
pub struct TerminalProgressSurface<W: Write> {
    sink: W,
    enabled: bool,
    painted_lines: usize,
    cursor_hidden: bool,
    finished: bool,
}

impl<W: Write> TerminalProgressSurface<W> {
    pub fn new(sink: W) -> Self {
        Self {
            sink,
            enabled: true,
            painted_lines: 0,
            cursor_hidden: false,
            finished: false,
        }
    }

    /// Creates an inert surface for callers that intentionally selected plain
    /// output. It emits no terminal-control bytes.
    #[cfg(test)]
    pub fn disabled(sink: W) -> Self {
        Self {
            sink,
            enabled: false,
            painted_lines: 0,
            cursor_hidden: false,
            finished: false,
        }
    }

    /// Queries the terminal for every repaint so a resize is reflected in the
    /// next frame. Terminal-query failures use the conservative block
    /// minimum rather than propagating a presentation-only failure.
    pub fn repaint(&mut self, snapshot: &[String]) -> std::io::Result<()> {
        let width = terminal::size()
            .map(|(columns, _)| usize::from(columns).max(1))
            .unwrap_or(80);
        self.repaint_with_width(snapshot, width)
    }

    /// Paints at an explicit width. This is useful for deterministic tests and
    /// for any future terminal abstraction; production callers use
    /// [`Self::repaint`] so the width is queried each time.
    pub fn repaint_with_width(&mut self, snapshot: &[String], width: usize) -> std::io::Result<()> {
        if !self.enabled || self.finished {
            return Ok(());
        }
        if !self.cursor_hidden {
            self.sink.queue(Hide)?;
            self.cursor_hidden = true;
        }
        self.clear_previous_frame()?;
        let lines = snapshot;
        for (index, line) in lines.iter().enumerate() {
            self.sink.queue(MoveToColumn(0))?;
            self.sink.queue(Clear(ClearType::CurrentLine))?;
            // Reserve the final column to avoid terminal auto-wrap corrupting
            // the fixed-height block, including after a terminal resize.
            let mut columns = 0;
            let clipped: String = line
                .chars()
                .take_while(|ch| {
                    columns += unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
                    columns <= width.saturating_sub(1)
                })
                .collect();
            self.sink.write_all(clipped.as_bytes())?;
            if index + 1 < lines.len() {
                self.sink.write_all(b"\n")?;
            }
        }
        self.sink.flush()?;
        self.painted_lines = lines.len();
        Ok(())
    }

    /// Clears the current block and makes the cursor visible so ordinary
    /// append-only diagnostics can be printed by the caller.
    pub fn suspend_for_output(&mut self) -> std::io::Result<()> {
        if !self.enabled || self.finished {
            return Ok(());
        }
        self.clear_previous_frame()?;
        self.show_cursor()?;
        self.sink.flush()
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
/// deliberately ASCII and contain neither carriage-return blocks nor
/// cursor-control sequences.
pub struct PlainProgressReporter<W: Write> {
    sink: W,
}

impl<W: Write> PlainProgressReporter<W> {
    pub fn new(sink: W) -> Self {
        Self { sink }
    }

    pub fn report(&mut self, snapshot: &[String]) -> std::io::Result<()> {
        let line = snapshot.join("; ");
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
    use super::*;

    #[test]
    fn terminal_surface_clips_rows_by_display_columns_and_reserves_last_column() {
        let mut block = format_progress_block(&test_stages(), false, std::time::Duration::ZERO, 0);
        block.push("界界界界界界界界界界界".into());
        let mut surface = TerminalProgressSurface::new(Vec::new());
        surface.repaint_with_width(&block, 20).unwrap();
        let output = std::str::from_utf8(surface.sink()).unwrap();
        let mut visible = String::new();
        let mut chars = output.chars();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                assert_eq!(chars.next(), Some('['));
                for control in chars.by_ref() {
                    if ('@'..='~').contains(&control) {
                        break;
                    }
                }
            } else {
                visible.push(ch);
            }
        }
        assert_eq!(visible.lines().count(), block.len());
        for row in visible.lines() {
            assert!(unicode_width::UnicodeWidthStr::width(row) <= 19, "{row:?}");
        }
        assert_eq!(visible.lines().last(), Some("界界界界界界界界界"));
    }

    #[test]
    fn plain_and_disabled_terminal_surfaces_emit_no_cursor_control_bytes() {
        let snapshot = format_progress_block(&test_stages(), false, std::time::Duration::ZERO, 0);
        let mut plain = PlainProgressReporter::new(Vec::new());
        plain.report(&snapshot).unwrap();
        let plain = std::str::from_utf8(plain.sink()).unwrap();
        assert!(!plain.contains('\u{1b}') && !plain.contains('\r'));

        let mut disabled = TerminalProgressSurface::disabled(Vec::new());
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
        let snapshot = format_progress_block(&test_stages(), false, std::time::Duration::ZERO, 0);
        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        {
            let mut surface = TerminalProgressSurface::new(shared.clone());
            surface.repaint_with_width(&snapshot, 140).unwrap();
        }
        let dropped = shared.0.lock().unwrap().clone();
        assert!(dropped.windows(6).any(|bytes| bytes == b"\x1b[?25h"));
        assert!(dropped.ends_with(b"\n"));

        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        let mut surface = TerminalProgressSurface::new(shared.clone());
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
        let first = format_progress_block(&test_stages(), false, std::time::Duration::ZERO, 0);

        let second = format_progress_block(&test_stages(), false, std::time::Duration::ZERO, 0);
        let shared = SharedOutput(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        let mut surface = TerminalProgressSurface::new(shared.clone());

        surface.repaint_with_width(&first, 140).unwrap();
        surface.repaint_with_width(&second, 60).unwrap();

        let output = String::from_utf8(shared.0.lock().unwrap().clone()).unwrap();
        // The first wide block is six rows; the second repaint must move
        // back to its first row and clear every previous row before drawing a
        // compact replacement block.
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
