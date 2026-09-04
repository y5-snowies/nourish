//! The spawn door: the one place a `Command` is turned into a process, so the two things
//! every spawn has to settle — whether the program could be resolved before forking, and
//! who will wait on the child — are stated once instead of at each call site.

pub mod spawn;
