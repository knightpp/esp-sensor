#![no_std]
#![feature(impl_trait_projections)] // http_compat
#![feature(async_fn_in_trait)] // http_compat
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

extern crate alloc;

pub mod http_compat;
pub mod line_proto;
