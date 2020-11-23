use std::fmt;

#[derive(Copy, Clone)]
pub enum ErrorType{
	InvalidRequest,
    ClientTimeout,
    VideoSenderClosed,
}

pub struct Error{
	pub error_type: ErrorType,
}

impl Error {
    fn format(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorType::*;
        write!(f, "{}", match self.error_type {
            InvalidRequest => "invalid_request",
            ClientTimeout => "client_timeout",
            VideoSenderClosed => "video_sender_closed",
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    	self.format(f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    	self.format(f)
    }
}

impl std::error::Error for Error{}

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
