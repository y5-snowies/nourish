//! The spawn door: the one place a `Command` is turned into a process, so the
//! two things every spawn owes the SIGCHLD reaper — resolving the program
//! before forking, and excluding the reaper for the fork+exec window — are
//! stated once instead of at each call site.

pub mod spawn;
