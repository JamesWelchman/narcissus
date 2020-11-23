use std::sync::{Arc, Mutex};
use std::path::Path;
use std::fs::remove_file;
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread::{JoinHandle, Builder, sleep};
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::time;

use crate::errors::*;
use crate::narcissus::Narcissus;
use crate::exchange::Exchange;
use crate::{info, error, tags};

use super::session::Session;

pub struct Server{
	n: Arc<Narcissus>,
	exc: Arc<Mutex<Exchange>>,
	listener: UnixListener,
	client_num: u32,

	// A vector of (handle, channel) pairs
	// to wait for our client threads to close
	clients: Vec<(Option<JoinHandle<()>>, Sender<()>)>,
}

impl Server {
	pub fn new(n: Arc<Narcissus>, exc: Arc<Mutex<Exchange>>) 
		-> Result<Self> {

		let path = Path::new(&n.config.socket_path);
		if path.exists() {
			remove_file(&path)?;
		}

		// Create the Unix socket file
		info!("creating unix socket", tags![
			("path", &n.config.socket_path)
		]);
		let listener = UnixListener::bind(path)?;
		listener.set_nonblocking(true)?;

		Ok(Self{
			n: n,
			exc: exc,
			listener: listener,
			client_num: 0,
			clients: vec![],
		})
	}

	pub fn tick(&mut self) -> Result<()> {
		// Threading server - check if we have
		// any new client connections
		use std::io::ErrorKind::WouldBlock;

		match self.listener.accept() {
			Ok((stream, _)) => {
				// Spawn a new thread
				let name = format!("client_{}", self.client_num);
				self.client_num += 1;
				let (sender, receiver) = channel();

				let n = self.n.clone();
				let e = self.exc.clone();

				let handle = Builder::new()
					.name(name.clone())
					.spawn(|| start_session(n, e, stream, receiver))?;

				// Add this thread to our Vector
				self.clients.push((Some(handle), sender));
				Ok(())
			},
			Err(ref e) if e.kind() == WouldBlock => Ok(()),
			Err(e) => Err(e),
		}?;

		// TODO: Poll our client threads to see if any of them
		// need removing from our vector.

		Ok(())
	}

	pub fn shutdown(&mut self) -> Result<()> {
		// Send shutdown to all the clients
		for (handle, sender) in self.clients.iter_mut() {
			if let Err(e) = sender.send(()) {
				error!("couldn't send close to client thread", tags![
					("error", &e.to_string())
				]);
				continue;
			}

			if let Some(handle) = handle.take() {
				handle.join().expect("couldn't join on client thread");
			}
		}

		Ok(())
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		if let Err(e) = remove_file(&self.n.config.socket_path) {
			error!("couldn't remove socket file", tags![
				("error", &e.to_string())
			]);
		}
	}
}

fn start_session(n: Arc<Narcissus>,
	            exc: Arc<Mutex<Exchange>>,
	            stream: UnixStream,
	            closer: Receiver<()>) {
	info!("new session");
	if let Err(e) = run_session(n, exc, stream, closer) {
		error!("session crashed", tags![
			("error", &e.to_string())
		]);
	}
}

fn run_session(n: Arc<Narcissus>,
	          exc: Arc<Mutex<Exchange>>,
	          stream: UnixStream,
	          closer: Receiver<()>) -> Result<()> {

	// Create our client
	let mut c = Session::new(n, exc, stream)?;

	// Block here waiting for client hello
	// This will timeout and Error so the
	// client can't hang.
	c.read_hello()?;

	// Okay send server hello back
	c.write_hello()?;

	c.info("session established");

	loop {
		// Poll the channel to check if we're shutting down
		match closer.try_recv() {
			Ok(_) => {
				// Send a shutdown to the client
				c.info("sending shutdown");
				c.shutdown()?;
				break;
			},
			Err(TryRecvError::Empty) => Ok(()),
			Err(e) => Err(e),
		}?;

		// tick_read returns false to close
		// This occurs when the client has
		// instaniated the shutdown.
		if !c.tick_read()? {
			break;
		}

		// tick_write can also potentially call shutdown
		// this occurs when the client has stopped heart
		// beating. We assume it's dead and stop streaming.
		c.tick_write()?;

		// We need to throttle slightly to prevent our CPU
		// being eaten.
		sleep(time::Duration::from_millis(20));
	}
	c.info("session finished");
	Ok(())
}
