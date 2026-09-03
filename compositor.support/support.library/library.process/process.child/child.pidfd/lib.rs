//! A file descriptor per child, so a child's exit is an ordinary event on the loop
//! rather than a signal plus an indiscriminate wait.

// Developer logging: error!/warn!/info!/trace!/abort! in scope for this crate.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod pidfd;
