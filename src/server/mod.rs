use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::thread::{Builder, JoinHandle, sleep};
use std::time;

use crate::errors::*;
use crate::narcissus::Narcissus;
use crate::exchange::Exchange;
use crate::{info, error, tags};

mod server;
use server::Server;
mod session;

pub struct ServerRAII{
	// Hold join handles and close channels
	handle: Option<JoinHandle<()>>,
	close_channel: Sender<()>,
}

impl ServerRAII {
	pub fn new(n: Arc<Narcissus>, exc: Exchange) -> Result<Self> {
		// Create thread for server
		let (sender, receiver) = channel();

		let handle = Builder::new()
			.name("server".to_string())
			.spawn(move || start_server(n, exc, receiver))?;

		Ok(Self{
			handle: Some(handle),
			close_channel: sender,
		})
	}
}

impl Drop for ServerRAII {
	fn drop(&mut self) {
		if let Err(e) = self.close_channel.send(()) {
			error!("couldn't close server", tags![
				("error", &e.to_string())
			]);
		}
		if let Some(handle) = self.handle.take() {
			handle.join()
				.expect("couldn't join on server thread");
		}
	}
}

fn start_server(n: Arc<Narcissus>,
			  exc: Exchange,
			  closer: Receiver<()>) {

	let exc = Arc::new(Mutex::new(exc));

	// Create our Server objects
	loop {
		if let Err(e) = run_server(n.clone(), exc.clone(), &closer) {
			error!("server crashed - restarting", tags![
				("error", &e.to_string())
			]);
		} else {
			return;
		}

		// Edge case - make sure we don't have a close message
		match closer.try_recv() {
			Ok(_) => {
				// We're closing anyway
				break;
			},
			Err(TryRecvError::Disconnected) => {
				break;
			},
			Err(TryRecvError::Empty) => {},
		}
	}
}

fn run_server(n: Arc<Narcissus>,
			  exc: Arc<Mutex<Exchange>>,
			  closer: &Receiver<()>) -> Result<()> {

	let mut server = Server::new(n, exc)?;

	loop {
		match closer.try_recv() {
			Ok(_) => {
				// We swallow + log errors from shutdown
				// This is because we don't want the start_server
				// function above to restart us. We stop accepting
				// client connections because we're not going to
				// call tick again.
				if let Err(e) = server.shutdown() {
					error!("something went wrong", tags![
						("error", &e.to_string())
					]);
				}
				break;
			},
			Err(TryRecvError::Disconnected) => {
				break;
			},
			Err(TryRecvError::Empty) => {},
		}

		// Server::new and server.tick only report OS
		// errors so we return from this function and
		// attempt a restart. Individual client errors
		// shouldn't affect this thread. server.tick()
		// will thread per client connection.
		server.tick()?;

		// We need to throttle to prevent our CPU being eaten
		sleep(time::Duration::from_millis(50));
	}

	info!("shutdown complete");
	Ok(())
}
