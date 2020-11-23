
fn main() {
	cc::Build::new()
		.file("src/videoq/videoq.c")
		.flag("--std=c99")
		.compile("videoq");

	cc::Build::new()
		.file("src/exchange/normalize_integral_image.c")
		.flag("--std=c99")
		.compile("normalize_integral_image");
}