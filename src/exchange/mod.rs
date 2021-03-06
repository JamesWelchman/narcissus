use std::sync::{Arc, Mutex};
use std::thread::{Builder, sleep};
use std::time::Duration;

extern crate rustface;
use rustface::ImageData;

use crate::errors::*;
use crate::videoq;
use crate::narcissus::Narcissus;

pub mod confchannel;
use confchannel::Sender;
pub mod msgs;
use msgs::*;

#[allow(dead_code)]
pub struct Exchange{
	receiver: videoq::Receiver,
	n: Arc<Narcissus>,

	// Our receivers
	// NOTE: An important part of our design is that
	// the mutex only locks the Senders. The Receivers
	// are still active when these mutexes are locked.
	faceposition_senders: 
		Arc<Mutex<Vec<confchannel::Sender<msgs::FacePosition>>>>,

	luminosity_senders: 
		Arc<Mutex<Vec<confchannel::Sender<msgs::Luminosity>>>>,
}

impl Exchange {
	pub fn new(n: Arc<Narcissus>, receiver: videoq::Receiver) 
		-> Result<Self> {

		// Face position
		let faceposition_senders = Arc::new(Mutex::new(vec![]));
		let f = faceposition_senders.clone();
		let n1 = n.clone();
		let r = receiver.clone();
		Builder::new()
			.name("faceposition".to_string())
			.spawn(move || faceposition(n1, r, f))?;

		// Luminosity
		let luminosity_senders = Arc::new(Mutex::new(vec![]));
		let n1 = n.clone();
		let r = receiver.clone();
		let l = luminosity_senders.clone();
		Builder::new()
			.name("luminosity".to_string())
			.spawn(move || luminosity(n1, r, l))?;

		Ok(Self{
			receiver: receiver,
			n: n,
			faceposition_senders: faceposition_senders,
			luminosity_senders: luminosity_senders,
		})
	}

	pub fn subscribe_faceposition(&self)
		-> confchannel::Receiver<FacePosition> {

		let mut senders = self.faceposition_senders.lock()
			.expect("couldn't lock faceposition mutex");

		let (sx, rx) = confchannel::confchannel();

		senders.push(sx);

		rx
	}

	pub fn subscribe_luminosity(&self)
		-> confchannel::Receiver<Luminosity> {

		let mut senders = self.luminosity_senders.lock()
			.expect("couldn't lock luminosity mutex");

		let (sx, rx) = confchannel::confchannel();

		senders.push(sx);

		rx

	}
}

fn faceposition(n: Arc<Narcissus>,
				receiver: videoq::Receiver,
				faceposition_senders: Arc<Mutex<Vec<Sender<FacePosition>>>>) {
	let mut faceposition = FacePosition::default();
	let mut to_delete = vec![];
	let mut no_subscribers = true;
	let (width, height) = (
		n.config.webcam_resolution.0, n.config.webcam_resolution.1
	);
	let num_lumin_bytes = (width * height) as usize;
	let mut old_timestamp: u64 = 0;

	// Face detection
	let mut grayscale = vec![0 as u8; num_lumin_bytes];
	let mut detector = rustface::create_detector("seeta_fd_frontal_v1.0.bin")
		.expect("couldn't read face detection model");

	loop {
		if no_subscribers {
			sleep(Duration::new(1, 0));
		}

		// Write to our senders
		{
			let mut senders = faceposition_senders.lock()
				.expect("couldn't lock faceposition mutex");

			if senders.len() > 0 {
				no_subscribers = false;
			} else {
				no_subscribers = true;
				continue;
			}

			to_delete.clear();
			for (n, s) in senders.iter_mut().enumerate() {
				let num_receivers = s.send(faceposition);
				if num_receivers == 0 {
					to_delete.push(n);
				}
			}

			// Delete any unused senders
			for (n, x) in to_delete.iter().enumerate() {
				senders.remove(x - n);
			}
		// Unlock the mutex around our subscribers vector
		}

		{

			// Grab a video frame
			let (frame, timestamp) = match receiver.recv() {
				Ok((frame, timestamp)) => (frame, timestamp),
				Err(_) => {
					// TODO: log
					break;
				},
			};

			if timestamp == faceposition.timestamp {
				// Already processed
				sleep(Duration::from_millis(20));
			}

			old_timestamp = faceposition.timestamp;
			faceposition.timestamp = timestamp;

			// Copy the lumin bytes
			frame.iter().step_by(2)
				.zip(grayscale.iter_mut())
				.for_each(|(&p, q)| *q = p);

		// Drop the frame
		}

		let mut image = ImageData::new(&grayscale, width, height);
		let mut size = 0;
		let mut found = false;
		for face in detector.detect(&mut image).into_iter() {
			found = true;
			// Use the biggest face
			let bbox = face.bbox();
			if (bbox.height() * bbox.width()) > size {
				faceposition.bottom_left = [
					if bbox.x() > 0 {bbox.x() as u32} else {0},
					if bbox.y() > 0 {bbox.y() as u32} else {0},
				];

				faceposition.top_right = [
					if bbox.x() > 0 {bbox.x() as u32} else {0}
					+ bbox.width(),
					if bbox.y() > 0 {bbox.y() as u32} else {0}
					+ bbox.height(),
				];
				size = bbox.height() * bbox.width();
			}
		}

		if !found {
			// If we don't find any faces then use
			// the old timestamp
			faceposition.timestamp = old_timestamp;
		}

	}
}

fn luminosity(n: Arc<Narcissus>,
			  receiver: videoq::Receiver,
			  luminosity_senders: Arc<Mutex<Vec<Sender<Luminosity>>>>) {
	let mut no_subscribers = true;
	let mut luminosity = Luminosity::default();
	let mut to_delete = vec![];
	let num_lumin_bytes = (
		n.config.webcam_resolution.0 * n.config.webcam_resolution.1
	) as f32;

	loop {
		if no_subscribers {
			sleep(Duration::from_secs(1));
		}

		// Grab a video frame
		let (frame, timestamp) = match receiver.recv() {
			Ok((frame, timestamp)) => (frame, timestamp),
			Err(_) => {
				// TODO: log
				break;
			},
		};

		if timestamp == luminosity.timestamp {
			// Already processed
			sleep(Duration::from_millis(20));
			continue;
		}

		// Lock the mutex and write to our senders
		{
			let mut senders = luminosity_senders.lock()
				.expect("couldn't lock faceposition mutex");
			if senders.len() > 0 {
				no_subscribers = false;
			} else {
				no_subscribers = true;
				continue;
			}

			to_delete.clear();
			for (n, s) in senders.iter_mut().enumerate() {
				let num_receivers = s.send(luminosity);
				if num_receivers == 0 {
					to_delete.push(n);
				}
			}

			// Delete any unused senders
			for (n, x) in to_delete.iter().enumerate() {
				senders.remove(x - n);
			}
		// Unlock the mutex around our subscribers vector
		}


		// Set the timestamp
		luminosity.timestamp = timestamp;

		luminosity.average = frame
			.iter()
			.step_by(2)
			.map(|&x| (x as f32) / num_lumin_bytes)
			.sum::<f32>();

		// Variance
		luminosity.standard_deviation = frame
			.iter()
			.step_by(2)
			.map(|&x| {
				((x as f32) - luminosity.average).powf(2.0)
			})
			.sum::<f32>() 
			.sqrt()
			/ num_lumin_bytes;

		let max = frame.iter().step_by(2).max();
		if let Some(max) = max {
			luminosity.max = *max as f32;
		}

		let min = frame.iter().step_by(2).min();
		if let Some(min) = min {
			luminosity.min = *min as f32;
		}
	}
}
