#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
pub mod platform;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));