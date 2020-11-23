// Key value logging macros

use std::thread;

pub type Tags<'a> = Vec<(&'static str, &'a str)>;

// The tags macro is essentially the same as vec![]
// Where the elements are of type (&'static str, String)

#[macro_export]
macro_rules! tags {
	// Tags is passed an array of (&'static str, String)
	($($x:expr),*) => {
		vec![
			$($x),*
		]
	};
}

#[macro_export]
macro_rules! info {
	// A Single Expression
	($msg:expr) => {
		use crate::ltsv::{log, Tags};
		log("info", $msg, Tags::new());
	};
	// An expression with tags
	($msg:expr, $kvs:expr) => {
		use crate::ltsv::log;
		log("info", $msg, $kvs);
	};
}


#[macro_export]
macro_rules! error {
	// A Single Expression
	($msg:expr) => {
		use crate::ltsv::{log, Tags};
		log("error", $msg, Tags::new());
	};
	// An expression with tags
	($msg:expr, $kvs:expr) => {
		use crate::ltsv::log;
		log("error", $msg, $kvs);
	};
}

fn ltsv_print(mut log_line: String, vals: &[(&str, &str)]) {
	for (key, value) in vals.iter() {
		log_line.push('\t');
		ltsv_encode(&mut log_line, key, value);
	}
	println!("{}", log_line);
}

fn ltsv_encode(buf: &mut String, key: &str, value: &str) {
	// Encode the key
	for c in key.chars() {
		if c == '\\' || c == '\n' || c == '\t' || c == '=' {
			buf.push('\\');
		}
		buf.push(c)
	}

	buf.push('=');

	for c in value.chars() {
		if c == '\\' || c == '\n' || c == '\t' || c == '=' {
			buf.push('\\');
		}
		buf.push(c)
	}
}

pub fn log(level: &'static str,
	       msg: &str,
	       tags: Tags) {
	let mut log_line = String::with_capacity(1024);
	// The first entry is the thread name
	ltsv_encode(&mut log_line, "thread",
				thread::current()
		        	.name()
		        	.expect("couldn't get thread name"));

	// The second tag is the level
	log_line.push('\t');
	ltsv_encode(&mut log_line, "level", level);

	// The third tag is the message
	log_line.push('\t');
	ltsv_encode(&mut log_line, "msg", msg);

	// We add any additional tags
	ltsv_print(log_line, &tags);
}