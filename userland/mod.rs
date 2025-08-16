//! Userland Module
//!
//! Contains user-space programs, libraries, and test infrastructure.

pub mod test_programs;
pub mod test_runner;

pub use test_runner::{
    run_phase2_validation, 
    run_critical_tests, 
    run_specific_tests, 
    interactive_test_menu,
    TestSuiteSummary
};