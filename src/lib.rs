#[macro_use]
extern crate failure;

pub type Result<T> = std::result::Result<T, failure::Error>;
