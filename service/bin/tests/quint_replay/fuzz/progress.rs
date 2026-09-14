use std::{
	sync::Mutex,
	time::{Duration, Instant},
};

/// A single campaign-wide reporter; workers count only successfully replayed traces.
pub(super) struct Progress {
	started: Instant,
	total: u64,
	steps: u64,
	counts: Mutex<Counts>,
}

struct Counts {
	completed: u64,
	reported: u64,
	reported_at: Instant,
}

impl Progress {
	pub(super) fn new(total: u64, steps: u64) -> Self {
		let started = Instant::now();
		Self {
			started,
			total,
			steps,
			counts: Mutex::new(Counts { completed: 0, reported: 0, reported_at: started }),
		}
	}

	pub(super) fn completed(&self) {
		let mut counts = self.counts.lock().expect("progress lock poisoned");
		counts.completed += 1;
		let now = Instant::now();
		let interval = now.duration_since(counts.reported_at);
		if interval >= Duration::from_secs(5) {
			self.print(
				"running",
				counts.completed,
				(counts.completed - counts.reported) as f64 / interval.as_secs_f64(),
				false,
			);
			counts.reported = counts.completed;
			counts.reported_at = now;
		}
	}

	pub(super) fn finish(&self, success: bool) {
		let counts = self.counts.lock().expect("progress lock poisoned");
		self.print(
			if success { "done" } else { "FAILED" },
			counts.completed,
			counts.completed as f64 / self.started.elapsed().as_secs_f64().max(f64::EPSILON),
			true,
		);
	}

	fn print(&self, status: &str, completed: u64, traces_per_second: f64, average: bool) {
		let progress = if self.total == 0 {
			completed.to_string()
		} else {
			format!("{completed}/{}", self.total)
		};
		let seconds = self.started.elapsed().as_secs();
		let rate = if average { " avg" } else { "" };
		eprintln!(
			"Quint fuzz {status} | {traces_per_second:.1} traces/s{rate} | {:.1} transitions/s{rate} | {progress} traces | {} transitions | {:02}:{:02}:{:02} elapsed",
			traces_per_second * self.steps as f64,
			u128::from(completed) * u128::from(self.steps),
			seconds / 3600,
			seconds / 60 % 60,
			seconds % 60,
		);
	}
}
