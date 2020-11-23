use std::sync::{Arc, Mutex};
use std::os::unix::net::UnixStream;
use std::time;
use std::io::{Read, Write};
use std::fs::{File, OpenOptions};

use serde::{Serialize, Deserialize};

use crate::errors::*;
use crate::narcissus::{Narcissus, Config};
use crate::exchange::Exchange;
use crate::exchange::confchannel::Receiver;
use crate::exchange::msgs::{FacePosition, Luminosity};
use crate::{info, error, tags};

#[derive(Copy, Clone, PartialEq)]
enum ReadState {
	Header,
	Body,
}

#[derive(Serialize)]
struct Empty{}

const VERSION: u8 = 0;


pub struct Session{
	n: Arc<Narcissus>,
	exc: Arc<Mutex<Exchange>>,
	stream: UnixStream,
	last_read: time::Instant,

	// Our receivers, clients may subscribe to these
	faceposition_receiver: Option<Receiver<FacePosition>>,
	faceposition_last_write: time::Instant,
	faceposition_update_rate: time::Duration,

	luminosity_receiver: Option<Receiver<Luminosity>>,
	luminosity_last_write: time::Instant,
	luminosity_update_rate: time::Duration,

	// Session Data
	session_id: String,

	// Read state / buffers
	read_state: ReadState,
	read_header_buf: [u8; 10],
	read_bytes_read: usize,
	read_body_buf: Vec<u8>,
	read_header: Header,

	// Write buffers / state
	write_buffer: Vec<u8>,
	write_msg_id: u32,

	// Random number file/buffer
	rand_file: File,
	rand_buf: [u8; 4],
}

impl Session {
	pub fn new(n: Arc<Narcissus>,
		exc: Arc<Mutex<Exchange>>,
		stream: UnixStream) -> Result<Self>{

		let rand_file = OpenOptions::new()
			.read(true)
			.open("/dev/random")?;

		Ok(Self{
			n: n,
			exc: exc,
			stream: stream,
			last_read: time::Instant::now(),
			faceposition_receiver: None,
			faceposition_last_write: time::Instant::now(),
			faceposition_update_rate: time::Duration::new(1, 0),
			luminosity_receiver: None,
			luminosity_last_write: time::Instant::now(),
			luminosity_update_rate: time::Duration::new(1, 0),
			session_id: String::new(),
			read_state: ReadState::Header,
			read_header_buf: [0; 10],
			read_bytes_read: 0,
			read_body_buf: Vec::with_capacity(1024),
			read_header: Header::default(),
			write_buffer: Vec::with_capacity(1024),
			write_msg_id: 0,
			rand_file: rand_file,
			rand_buf: [0; 4],
		})
	}

	fn subscribe_faceposition(&mut self, req: FacepositionRequest) {
		info!("subscribing to faceposition", tags![
			("session_id", &self.session_id),
			("update_interval", &format!("{}", req.update_interval))
		]);
		// If we already have a subscription
		// then we overwrite with the new
		// params from the client.
		self.faceposition_receiver.take();

		if req.update_interval == 0 {
			// This is our protocol for stopping streaming.
			// The take above has already dropped the Receiver
			return;
		}

		use time::Duration;
		let millis = req.update_interval as u64;
		self.faceposition_update_rate = Duration::from_millis(millis);

		self.faceposition_receiver = Some({
			let exc = self.exc.lock()
				.expect("couldn't lock exc mutex");
			exc.subscribe_faceposition()
		});
	}

	fn subscribe_luminosity(&mut self, req: LuminosityRequest) {
		info!("subscribing to luminosity", tags![
			("session_id", &self.session_id),
			("update_interval", &format!("{}", req.update_interval))
		]);
		// If we already have a subscription
		// then we overwrite with the new
		// params from the client.
		self.luminosity_receiver.take();

		if req.update_interval == 0 {
			// This is our protocol for stopping streaming.
			// The take above has already dropped the Receiver
			return;
		}

		use time::Duration;
		let millis = req.update_interval as u64;
		self.luminosity_update_rate = Duration::from_millis(millis);

		self.luminosity_receiver = Some({
			let exc = self.exc.lock()
				.expect("couldn't lock exc mutex");
			exc.subscribe_luminosity()
		});
	}

	fn rand_bytes(&mut self) -> Result<()> {
		self.rand_file.read_exact(&mut self.rand_buf)?;
		Ok(())
	}

	fn new_session_id(&mut self) -> Result<()> {
		self.rand_bytes()?;
		let id = u32::from_le_bytes(self.rand_buf);
		self.session_id = format!("{:08x}", id);
		Ok(())
	}

	fn new_msg_id(&mut self) -> Result<u32> {
		self.rand_bytes()?;
		let id = u32::from_le_bytes(self.rand_buf);
		Ok(id)
	}

	fn write_msg<T: Serialize>(&mut self,
				               msg_type: MsgType,
				               body: &T) -> Result<()> {
		// push the version
		self.write_buffer.clear();
		self.write_buffer.push(VERSION);
		self.write_buffer.push(match msg_type {
			MsgType::Empty => unreachable!(),
			MsgType::Hello => b'a',
			MsgType::Shutdown => b'z',
			MsgType::Faceposition => b'f',
			MsgType::Luminosity => b'l',
			// Heartbeats have no response
			MsgType::Heartbeat => unreachable!()
		});

		// Serialize the body
		let body = serde_json::to_string(body)?;
		let len = body.len() as u32;

		// Generate a message id
		self.write_msg_id = self.new_msg_id()?;
		let msg_id = self.write_msg_id.to_le_bytes();
		self.write_buffer.extend_from_slice(&len.to_le_bytes());
		self.write_buffer.extend_from_slice(&msg_id);
		self.write_buffer.extend_from_slice(body.as_bytes());

		Ok(())
	}

	fn write(&mut self) -> Result<()> {
		use std::io::ErrorKind::WouldBlock;

		let mut num_sent = 0;
		while num_sent < self.write_buffer.len() {
			let buf = &self.write_buffer[num_sent..];
			num_sent += match self.stream.write(buf) {
				Ok(n) => Ok(n),
				Err(ref e) if e.kind() == WouldBlock => Ok(0),
				Err(e) => {
					error!("couldn't write to socket", tags![
						("error", &e.to_string())
					]);
					Err(e)
				},
			}?;
		}
		Ok(())
	}

	fn tick_read_header(&mut self) -> Result<bool> {
		{
			use std::io::ErrorKind::WouldBlock;

			let buf = &mut self.read_header_buf[self.read_bytes_read..];
			let bytes_read = match self.stream.read(buf) {
				Ok(size) => Ok(size),
				Err(ref e) if e.kind() == WouldBlock => Ok(0),
				Err(e) => {
					error!("couldn't read from socket");
					Err(e)
				},
			}?;

			if bytes_read == 0 {
				return Ok(true);
			}
			self.read_bytes_read += bytes_read;
		// Drop buf
		}

		if self.read_bytes_read == 10 {
			// Parse the header
			self.read_header = Header::from_raw(&self.read_header_buf)?;

			info!("received message header", tags![
				("session_id", &self.session_id),
				("msg_id", &format!("{}", self.read_header.msg_id)),
				("msg_type", &format!("{:?}", self.read_header.msg_type)),
				("msg_len", &format!("{}", self.read_header.msg_len))
			]);

			// Send back a shutdown and return false to notify
			// that we are done.
			if self.read_header.msg_type == MsgType::Shutdown {
				self.shutdown()?;
				return Ok(false);
			}

			// If we have a body length then prepare to parse it
			if self.read_header.msg_len > 0 {
				self.read_state = ReadState::Body;
				let len = self.read_header.msg_len as usize;
				self.read_body_buf.resize(len, 0);
			} else {
				// Set this to zero to parse the next header
				self.read_bytes_read = 0;
			}

			// If it's a heartbeat then set our last_read
			// to ensure we keep our streams alive.
			if self.read_header.msg_type == MsgType::Heartbeat {
				self.last_read = time::Instant::now();
			}
		}

		Ok(true)
	}

	fn tick_read_body(&mut self) -> Result<bool> {
		let bytes_parsed;

		{
			use std::io::ErrorKind::WouldBlock;

			bytes_parsed = self.read_bytes_read - 10;

			let buf = &mut self.read_body_buf[bytes_parsed..];
			self.read_bytes_read += match self.stream.read(buf) {
				Ok(n) => Ok(n),
				Err(ref e) if e.kind() == WouldBlock => Ok(0),
				Err(e) => {
					error!("couldn't read from socket");
					Err(e)
				},
			}?;
		// Drop buf
		}

		// Have we got a complete message?
		if bytes_parsed as u32 == self.read_header.msg_len {
			info!("received body", tags![
				("session_id", &self.session_id),
				("msg_id", &format!("{}", self.read_header.msg_id)),
				("msg_type", &format!("{:?}", self.read_header.msg_type)),
				("msg_len", &format!("{}", self.read_header.msg_len))
			]);

			// We need to process this
			match self.read_header.msg_type {
				// A bunch of message have no body
				MsgType::Empty => unreachable!(),
				MsgType::Hello => unreachable!(),
				MsgType::Shutdown => unreachable!(),
				MsgType::Heartbeat => unreachable!(),
				MsgType::Faceposition => {
					let req: FacepositionRequest = 
						serde_json::from_slice(&self.read_body_buf)?;
					self.subscribe_faceposition(req);
				},
				MsgType::Luminosity => {
					let req: LuminosityRequest = 
						serde_json::from_slice(&self.read_body_buf)?;
					self.subscribe_luminosity(req);
				},
			}

			self.read_state = ReadState::Header;
			self.read_bytes_read = 0;
		}
		Ok(true)
	}

	pub fn read_hello(&mut self) -> Result<()> {
		// Read exactly ten bytes (i.e the header)
		use time::Duration;
		let t = Duration::new(self.n.config.client_hello_timeout, 0);
		self.stream.set_read_timeout(Some(t))?;
		self.stream.read_exact(&mut self.read_header_buf)?;
		self.read_header = Header::from_raw(
			&self.read_header_buf)?;

		if self.read_header.msg_type != MsgType::Hello {
			return Err(Box::new(Error{
				error_type: ErrorType::InvalidRequest,
			}));
		}

		// Check msg_len is zero
		if self.read_header.msg_len != 0 {
			return Err(Box::new(Error{
				error_type: ErrorType::InvalidRequest,
			}));
		}
		self.last_read = time::Instant::now();
		self.new_session_id()?;
		info!("received client hello", tags![
			("session_id", &self.session_id),
			("msg_id", &format!("{}", self.read_header.msg_id))
		]);

		Ok(())
	}

	pub fn write_hello(&mut self) -> Result<()> {
		let body = HelloResponse{
			config: self.n.config.clone(),
			session_id: self.session_id.clone(),
		};

		self.write_msg(MsgType::Hello, &body)?;
		self.write()?;
		Ok(())
	}

	pub fn info(&self, msg: &'static str) {
		info!(msg, tags![
			("session_id", &self.session_id)
		]);
	}

	pub fn shutdown(&mut self) -> Result<()> {
		// Send shutdown
		self.write_msg(MsgType::Shutdown, &Empty{})?;
		self.write()?;
		Ok(())
	}

	pub fn tick_read(&mut self) -> Result<bool> {
		self.stream.set_nonblocking(true)?;
		if self.read_state == ReadState::Header {
			self.tick_read_header()
		} else {
			self.tick_read_body()
		}
	}

	pub fn tick_write(&mut self) -> Result<()> {
		if self.last_read.elapsed() > time::Duration::new(15, 0) {
			// The client has gone away
			// Try to shutdown but the client is probably dead
			self.info("closing due to timeout");
			self.shutdown()?;
			return Err(Box::new(Error{
				error_type: ErrorType::ClientTimeout,
			}));
		}

		let now = time::Instant::now();

		// Check if we're subscribed to and enough time has
		// elapsed to send a faceposition update.
		if let Some(ref receiver) = self.faceposition_receiver {
			let fp_elapsed = now - self.faceposition_last_write;
			if fp_elapsed > self.faceposition_update_rate {
				if let Some(fp) = receiver.recv() {
					// Write facepos to the client
					self.write_msg(MsgType::Faceposition, &fp)?;
					self.write()?;

					self.faceposition_last_write = now;
				}
			}
		}

		// Check if we're subscribed to and enough time has
		// elapsed to send a luminosity update.
		if let Some(ref receiver) = self.luminosity_receiver {
			let l_elapsed = now - self.luminosity_last_write;
			if l_elapsed > self.luminosity_update_rate {
				if let Some(l) = receiver.recv() {
					// Write luminosity to the client
					self.write_msg(MsgType::Luminosity, &l)?;
					self.write()?;

					self.luminosity_last_write = now;
				}
			}
		}

		Ok(())
	}
}


#[derive(Copy, Clone, PartialEq, Debug)]
enum MsgType {
	Empty,
	Hello,
	Shutdown,
	Heartbeat,
	Faceposition,
	Luminosity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HelloResponse {
	config: Config,
	session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FacepositionRequest {
	update_interval: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LuminosityRequest {
	update_interval: u32,
}

impl Default for MsgType {
	fn default() -> Self {
		MsgType::Empty
	}
}

#[derive(Default)]
#[allow(dead_code)]
struct Header {
	version: u8,
	msg_type: MsgType,
	msg_len: u32,
	msg_id: u32,
}

impl Header {
	fn from_raw(raw: &[u8; 10]) -> Result<Self> {
		// The first byte is the version
		if raw[0] != 0 {
			return Err(Box::new(Error{
				error_type: ErrorType::InvalidRequest,
			}));
		}

		// Okay read the msg_type
		let msg_type = match raw[1] {
			b'A' => Ok(MsgType::Hello),
			b'Z' => Ok(MsgType::Shutdown),
			b'H' => Ok(MsgType::Heartbeat),
			b'F' => Ok(MsgType::Faceposition),
			b'L' => Ok(MsgType::Luminosity),
			_ => {
				Err(Box::new(Error{
					error_type: ErrorType::InvalidRequest,
				}))
			},
		}?;

		// Parse the msg_len - u32 little endian
		let msg_len_buf = [raw[2], raw[3], raw[4], raw[5]];
		let msg_len = u32::from_le_bytes(msg_len_buf);

		let msg_id_buf = [raw[6], raw[7], raw[8], raw[9]];
		let msg_id = u32::from_le_bytes(msg_id_buf);

		Ok(Self{
			version: raw[0],
			msg_type: msg_type,
			msg_len: msg_len,
			msg_id: msg_id,
		})
	}
}