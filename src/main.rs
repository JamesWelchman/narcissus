use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::fs::{OpenOptions, remove_file};
use std::io::Write;
use std::time::Duration;
use std::thread;

use libc;

mod errors;
use errors::*;
mod narcissus;
use narcissus::Narcissus;
mod server;
use server::ServerRAII;
mod webcam;
mod exchange;
use exchange::Exchange;

mod ltsv;
mod videoq;

struct PidFile{}

impl PidFile {
	fn new() -> Result<Self> {
		let pid = unsafe {
			libc::getpid()
		};

		// Write to /tmp/narcissus.pid
		info!("creating pidfile", tags![
			("path", "/tmp/narcissus.pid")
		]);
		let mut file = OpenOptions::new()
			.create_new(true)
			.write(true)
			.open("/tmp/narcissus.pid")?;

		file.write(format!("{}", pid).as_bytes())?;
		Ok(Self{})
	}

}

impl Drop for PidFile {
	fn drop(&mut self) {
		// Try to delete the pidfile
		// log an error if we can't.
		if let Err(e) = remove_file("/tmp/narcissus.pid") {
			error!("couldn't delete pidfile", tags![
				("error", &e.to_string())
			]);
		}
	}
}

fn run() -> Result<()> {
	info!("narcissus started");
	let _pidfile = PidFile::new()?;
	let n = Arc::new(Narcissus::new()?);

	// Ctrl-C handler
	let running = Arc::new(AtomicBool::new(true));
	let r = running.clone();

	ctrlc::set_handler(move || {
		info!("received ctrlc - closing");
		r.store(false, Ordering::SeqCst);
	}).expect("couldn't set ctrl-c handler");

	// Start the webcam
	let video_receiver = webcam::webcam(&n)?;

	// The exchange takes the video_receiver
	// It allows for dynamic subscription
	// to it's metadata feeds.
	let exc = Exchange::new(n.clone(), video_receiver)?;

	// Start the threading server
	let _server_raii = ServerRAII::new(n.clone(), exc)?;

	// poll for shutdown twenty times per second
	while running.load(Ordering::SeqCst) {
		thread::sleep(Duration::from_millis(50));
	}

	Ok(())
}

fn main() {
	if let Err(e) = run() {
		error!("something went wrong", tags![
			("error", &e.to_string())
		]);
	}
}
