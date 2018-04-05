#![deny(unused_must_use)]

#[macro_use]
extern crate failure;
#[macro_use]
extern crate hyper;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;

pub static USER_AGENT: &str = concat!("rust-ops/rust-log-analyzer ", env!("CARGO_PKG_VERSION"));

pub mod travis;

pub type Result<T> = std::result::Result<T, failure::Error>;
