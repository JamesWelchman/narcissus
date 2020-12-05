
fn main() {
	cc::Build::new()
		.file("src/videoq/videoq.c")
		.flag("--std=c99")
		.compile("videoq");
}