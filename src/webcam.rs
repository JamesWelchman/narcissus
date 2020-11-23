use std::thread::Builder;

use rscam::Camera;

use crate::errors::*;
use crate::{info, error, tags};
use crate::narcissus::Narcissus;
use crate::videoq;


pub fn webcam(n:&Narcissus) -> Result<videoq::Receiver> {
	// Open the camera
	info!("opening camera", tags![
		("webcam_device", &n.config.webcam_device),
		("webcam_interval", &format!("{:?}", &n.config.webcam_interval)),
		("webcam_resolution", &format!("{:?}", &n.config.webcam_resolution))
	]);
	let mut camera = Camera::new(&n.config.webcam_device)?;
	let config = rscam::Config{
		interval: n.config.webcam_interval,
		resolution: n.config.webcam_resolution,
		format: b"YUYV",
		nbuffers: 2,
		field: rscam::FIELD_NONE,
	};

	camera.start(&config)?;

	// Check it's working
	for _ in 0..3 {
		camera.capture()?;
	}

	let (sender, receiver) = videoq::videoq({
		n.config.webcam_resolution.0 *
		n.config.webcam_resolution.1 *
		2
	} as usize);

	// Spawn the thread
	Builder::new()
		.name("webcam".to_string())
		.spawn(move || {
			info!("capture started");
			webcam_run(camera, sender);
		})?;

	Ok(receiver)
}

fn webcam_run(camera: Camera,
			  sender: videoq::Sender) {

	loop {
		match camera.capture() {
			Err(e) => {
				error!("couldn't read frame", tags![
					("error", &e.to_string())
				]);
			},
			Ok(frame) => {
				// Send returns false if there are no
				// receivers.
				let b = sender.send(&frame[..], frame.get_timestamp());
				if !b {
					break;
				}
			},
		}
	}

	info!("thread closing");
}
