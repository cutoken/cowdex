use std::io::IsTerminal;
use std::time::Duration;
use std::time::Instant;

pub(super) struct Progress {
    files: usize,
    current: usize,
    records: u64,
    terminal: bool,
    last: Instant,
}

impl Progress {
    pub(super) fn new(files: usize) -> Self {
        let progress = Self {
            files,
            current: 0,
            records: 0,
            terminal: std::io::stderr().is_terminal(),
            last: Instant::now(),
        };
        progress.show();
        progress
    }

    pub(super) fn begin(&mut self, current: usize) {
        self.current = current;
    }

    pub(super) fn record(&mut self) {
        self.records += 1;
        if self.last.elapsed() >= Duration::from_secs(1) {
            self.show();
            self.last = Instant::now();
        }
    }

    pub(super) fn finish(self) {
        self.show();
        if self.terminal {
            eprintln!();
        }
    }

    fn show(&self) {
        let status = format!(
            "Reading usage: {}/{} files ({} records)",
            self.current, self.files, self.records
        );
        if self.terminal {
            eprint!("\r\x1b[2K{status}");
        } else {
            eprintln!("{status}");
        }
    }
}
