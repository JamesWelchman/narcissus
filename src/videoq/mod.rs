use std::slice;

use crate::errors::*;

#[repr(C)]
pub struct Sender{
	// ringq is the underlying data structure
	ringq: *const libc::c_void,
	bufsize: libc::size_t,
}

#[repr(C)]
pub struct Receiver{
	ringq: *const libc::c_void,
	bufsize: libc::size_t,
	index: libc::size_t,
	data_ptr: *mut u8,
	timestamp: u64,
}


#[repr(C)]
pub struct SenderReceiverPair {
	sender: Sender,
	receiver: Receiver,
}

#[link(name="videoq")]
extern {
	fn new_ringq(bufsize: libc::size_t) -> SenderReceiverPair;
	fn send(sender: *const Sender, data: *const u8, timestamp: u64
		) -> libc::c_int;
	fn free_sender(sender: *const Sender);
	fn start_recv(receiver: *const Receiver) -> libc::c_int;
	fn end_recv(receiver: *const Receiver) -> libc::c_int;
	fn new_receiver(receiver: *const Receiver, error: *mut libc::c_int
		) -> Receiver;
}


impl Sender {
	// Return False when there are no Receivers
	// This is how we "back-propogate" to close
	// the webcam connection.
	pub fn send(&self, data: &[u8], timestamp: u64) -> bool {
		assert_eq!(self.bufsize, data.len());
		let ret = unsafe {
			send(self, data.as_ptr(), timestamp)
		};
		if ret == 0 {
			true
		} else if ret == 1 {
			false
		} else {
			panic!("unrecognised videoq response code");
		}
	}
}

impl Drop for Sender {
	fn drop(&mut self) {
		unsafe {
			free_sender(self);
		}
	}
}

// Frame is an abstraction over data of type &[u8; bufsize]
// We need to implement Drop to remove the borrow.
// This corresponds to the start_recv and end_recv functions
// in our C code.
pub struct Frame<'a> {
	receiver: &'a Receiver,
}

impl<'a> std::ops::Deref for Frame<'a> {
	type Target = [u8];

	fn deref(&self) -> &Self::Target {
		unsafe {
			let data_ptr = self.receiver.data_ptr;
			let len = self.receiver.bufsize;
			slice::from_raw_parts(data_ptr, len)
		}
	}
}

impl<'a> Drop for Frame<'a> {
	// We need to free
	fn drop(&mut self) {
		unsafe {
			end_recv(self.receiver);
		};
	}
}

impl Receiver {
	pub fn recv(&self) -> Result<(Frame, u64)> {
		let ret = unsafe {
			start_recv(self)
		};
		if ret == 0 {
			Ok((Frame{receiver: self}, self.timestamp))
		} else if ret == 2 {
			// Sender closed
			Err(Box::new(Error{
				error_type: ErrorType::VideoSenderClosed
			}))
		} else {
			panic!("unrecognised return code in videoq")
		}
	}
}

impl Clone for Receiver {
	fn clone(&self) -> Self {
		let mut error: libc::c_int = 0;
		let r = unsafe {
			new_receiver(self, &mut error)
		};
		if error != 0 {
			// TODO: Is there a better way to handle
			// this error? try_clone maybe?
			panic!("receiver cloned too many times");
		}
		r
	}
}

pub fn videoq(size: usize) -> (Sender, Receiver) {
	let pair = unsafe {
		new_ringq(size)
	};

	(pair.sender, pair.receiver)
}

unsafe impl Send for Sender{}
unsafe impl Send for Receiver{}
