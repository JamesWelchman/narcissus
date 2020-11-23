use serde::Serialize;

#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacePosition {
	pub timestamp: u64,
	pub bottom_left: [u32; 2],
	pub top_right: [u32; 2],
}

#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Luminosity {
	pub timestamp: u64,
	pub average: f32,
	pub standard_deviation: f32,
	pub max: f32,
	pub min: f32,
}