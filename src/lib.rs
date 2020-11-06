//! RTRTR: a versatile tool for route filters.
//!
//! This is the library crate of RTRTR providing all real functionality.
//!
//! RTRTR is designed around a concept of components being plugged together
//! in whichever way the user likes. There are currently two types of
//! components: _units_ that collect, process, and forward data, and
//! _targets_ that make the data from a select unit available to the outside.
//! All types of units and targets are defined in the modules [units] and
//! [targets], respectively.
//!
//! The means for communication between components are provided by types
//! defined in [comms], the data exchanged between them in [payload]. 
//! Everything is held together by a [`Manager`](manager::Manager) defined
//! in [manager].
//!
//! In addition, a number of modules provide auxiliary functionality, such as
//! [config] and [log].
//!
//! If you are trying to get started with the source code, perhaps begin with
//! [comms] and continue with [units] before reading [manager]. This should
//! give you a somewhat gentle introduction into the overall architecture.
#![allow(clippy::unknown_clippy_lints)]

pub mod comms;
pub mod config;
pub mod http;
pub mod log;
pub mod manager;
pub mod metrics;
pub mod payload;
pub mod targets;
pub mod units;
