use crate::errors::*;

use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
	pub socket_path: String,
	pub webcam_device: String,
	pub webcam_interval: (u32, u32),
	pub webcam_resolution: (u32, u32),
	pub client_hello_timeout: u64,
}

// Narcissus is a global config passed around
// all threads.
pub struct Narcissus {
	pub config: Config,
}

impl Narcissus {
	pub fn new() -> Result<Self> {
		Ok(Self{
			config: Config {
				socket_path: "/tmp/narcissus.sock".to_string(),
				webcam_device: "/dev/video0".to_string(),
				webcam_interval: (1, 30),
				webcam_resolution: (640, 480),
				client_hello_timeout: 2,
			},
		})
	}
}
