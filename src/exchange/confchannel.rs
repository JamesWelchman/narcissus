//confchannel is a Conflation Channel
// Both send and recv are guarantted not
// to block. The API matches Rust channels.
// We can only have one Sender but can have
// any number of receivers.

use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering, AtomicU8};

struct Channel<T: Copy + Default> {
	data: [RwLock<T>; 2],
	dropped_sender: AtomicBool,
	ind: AtomicU8,
	num_receivers: AtomicU8,
}

pub struct Sender<T: Copy + Default>{
	chan: Arc<Channel<T>>,
	ind: u8,
}

pub struct Receiver<T: Copy + Default>{
	chan: Arc<Channel<T>>,
}

pub fn confchannel<T: Copy + Default>() -> (Sender<T>, Receiver<T>) {
	let chan = Arc::new(Channel{
		data: [RwLock::new(T::default()), RwLock::new(T::default())],
		dropped_sender: AtomicBool::new(false),
		ind: AtomicU8::new(0),
		num_receivers: AtomicU8::new(1),
	});

	(Sender{chan: chan.clone(), ind: 0}, Receiver{chan: chan})
}

impl<T: Copy + Default> Drop for Sender<T> {
	fn drop(&mut self) {
		self.chan.dropped_sender.store(true, Ordering::SeqCst);
	}
}

impl<T: Copy + Default> Sender<T> {
	pub fn send(&mut self, data: T) -> u8 {
		let mut x = if self.ind == 0 {
			self.chan.data[0].write()
				.expect("couldn't get confchannel lock")
		} else {
			self.chan.data[1].write()
				.expect("couldn't get confchannel lock")
		};

		*x = data;

		self.chan.ind.store(self.ind, Ordering::SeqCst);
		self.ind = (self.ind + 1) % 2;
		self.chan.num_receivers.load(Ordering::SeqCst)
	}
}

impl<T: Copy + Default> Receiver<T> {
	pub fn recv(&self) -> Option<T> {
		if self.chan.dropped_sender.load(Ordering::SeqCst) {
			return None;
		}
		let ind = self.chan.ind.load(Ordering::SeqCst);
		let x = self.chan.data[ind as usize].read()
			.expect("couldn't get confchannel lock");
		Some(*x)
	}
}

impl<T: Copy + Default> Clone for Receiver<T> {
	fn clone(&self) -> Self {
		self.chan.num_receivers.fetch_add(1, Ordering::SeqCst);
		Self{
			chan: self.chan.clone(),
		}
	}
}

impl<T: Copy + Default> Drop for Receiver<T> {
	fn drop(&mut self) {
		self.chan.num_receivers.fetch_sub(1, Ordering::SeqCst);
	}
}

impl<T: Copy + Default> Iterator for Receiver<T> {
	type Item = T;

	fn next(&mut self) -> Option<Self::Item> {
		self.recv()
	}
}